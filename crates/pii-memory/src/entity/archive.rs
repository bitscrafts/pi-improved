//! The audited removal path (spec 127): `archive` and `restore`.
//!
//! Nothing else in the store removes a record. `archive_before(t)` selects the records not read
//! since `t` (plus every relationship that touches a selected entity, so no link is left
//! dangling), writes them to `archive-<ts>.jsonl` sorted by id, **re-reads and verifies the
//! dump (bytes and line count)**, and only then removes them from the graph. If the dump cannot
//! be written and verified, nothing is removed and the refusal is counted and reported.
//!
//! The dump is plain JSONL, one record per line, readable by a human:
//! `{"type":"entity"|"relationship","record":{...},"access":{"last_access_secs":N,"access_count":N}}`.
//! It carries no timestamp inside, so two archives of the same state are byte-identical.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};

use pii_observe::{Event, Level, names};
use serde::Deserialize;
use serde_json::{Value, json};

use super::store::{
    Counters, EntityStore, MAX_FILE_BYTES, State, Written, bump, check_budget, io_err, read_json,
    write_atomic,
};
use super::tier::{Access, AccessFile, Tier};
use super::types::{
    Entity, EntityInput, GraphError, Reason, Relationship, RelationshipInput, entity_id,
    relationship_id,
};
use super::upsert::{upsert_entity, upsert_relationship};

/// What an archive wrote and removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReport {
    /// The dump, or `None` when nothing was old enough (no file is written for nothing).
    pub path: Option<PathBuf>,
    /// Records written and removed.
    pub records: u64,
    /// Of those, entities.
    pub entities: u64,
    /// Of those, relationships.
    pub relationships: u64,
    /// Bytes in the verified dump.
    pub bytes: u64,
}

/// What a restore re-established and what it refused.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestoreReport {
    /// Records created or merged.
    pub restored: u64,
    /// Records already present exactly as dumped.
    pub unchanged: u64,
    /// `(line, reason, detail)` for every line refused; the rest were still restored.
    pub refused: Vec<(usize, String, String)>,
}

#[derive(Deserialize)]
struct DumpAccess {
    #[serde(default)]
    access_count: u64,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    record: Value,
    access: Option<DumpAccess>,
}

fn failed(why: impl Into<String>) -> GraphError {
    GraphError::Refused(Reason::ArchiveFailed, why.into())
}

impl EntityStore {
    fn refuse_op(&self, op: &str, counters: &mut Counters, e: &GraphError) {
        bump(&mut counters.archive_ops, "refused");
        let reason = e.reason().unwrap_or("store");
        self.observer.event(
            &Event::new("memory", Level::Warn, names::MEMORY_ARCHIVE_REFUSED)
                .field("op", op)
                .field("reason", reason)
                .field("error", e.to_string()),
        );
    }

    /// Archives every record last read before `before_secs` (epoch seconds). See the module docs.
    ///
    /// # Errors
    /// `archive_failed` when the dump cannot be written or verified (nothing is removed); a
    /// store failure when a file cannot be read.
    pub fn archive_before(&self, before_secs: u64) -> Result<ArchiveReport, GraphError> {
        let _guard = self.lock()?;
        let mut state = self.load()?;
        let mut access: AccessFile = read_json(&self.access_path())?;
        let mut counters: Counters = read_json(&self.counters_path())?;
        let res = self.archive_locked(&mut state, &mut access, &mut counters, before_secs);
        if let Err(e @ GraphError::Refused(..)) = &res {
            self.refuse_op("archive", &mut counters, e);
        }
        let bytes = serde_json::to_vec(&counters).map_err(|e| GraphError::Store(e.to_string()))?;
        write_atomic(&self.counters_path(), &bytes)?;
        res
    }

    fn archive_locked(
        &self,
        state: &mut State,
        access: &mut AccessFile,
        counters: &mut Counters,
        before: u64,
    ) -> Result<ArchiveReport, GraphError> {
        let old = |id: &str| access.get(id).last_access_secs < before;
        let gone_e: Vec<String> = state
            .entities
            .keys()
            .filter(|id| old(id))
            .cloned()
            .collect();
        let gone_r: Vec<String> = state
            .relationships
            .values()
            .filter(|r| {
                old(&r.id)
                    || gone_e.binary_search(&r.from).is_ok()
                    || gone_e.binary_search(&r.to).is_ok()
            })
            .map(|r| r.id.clone())
            .collect();
        let records = (gone_e.len() + gone_r.len()) as u64;
        if records == 0 {
            return Ok(ArchiveReport {
                path: None,
                records: 0,
                entities: 0,
                relationships: 0,
                bytes: 0,
            });
        }
        // Sorted by id: entities (`e-`) before relationships (`r-`), each in id order.
        let mut lines: BTreeMap<&str, String> = BTreeMap::new();
        let dump = |kind: &str, id: &str, record: Value| {
            let a = access.get(id);
            json!({"type": kind, "record": record,
                   "access": {"last_access_secs": a.last_access_secs, "access_count": a.access_count}})
        };
        for id in &gone_e {
            if let Some(e) = state.entities.get(id) {
                lines.insert(id, dump("entity", id, json!(e)).to_string());
            }
        }
        for id in &gone_r {
            if let Some(r) = state.relationships.get(id) {
                lines.insert(id, dump("relationship", id, json!(r)).to_string());
            }
        }
        let mut content = String::new();
        for l in lines.values() {
            content.push_str(l);
            content.push('\n');
        }
        let path = self.area.join(format!("archive-{}.jsonl", self.now_secs()));
        write_verified(&path, &content, records)?;
        // The dump is on disk and verified: only now may anything leave the graph.
        for id in &gone_e {
            state.entities.remove(id);
            access.entries.remove(id);
        }
        for id in &gone_r {
            state.relationships.remove(id);
            access.entries.remove(id);
        }
        let g = serde_json::to_vec(&*state).map_err(|e| GraphError::Store(e.to_string()))?;
        write_atomic(&self.graph_path(), &g)?;
        let a = serde_json::to_vec(&*access).map_err(|e| GraphError::Store(e.to_string()))?;
        write_atomic(&self.access_path(), &a)?;
        bump(&mut counters.archive_ops, "archived");
        counters.archived_records = counters.archived_records.saturating_add(records);
        let report = ArchiveReport {
            path: Some(path.clone()),
            records,
            entities: gone_e.len() as u64,
            relationships: gone_r.len() as u64,
            bytes: content.len() as u64,
        };
        self.observer.event(
            &Event::new("memory", Level::Info, names::MEMORY_ARCHIVED)
                .field("path", path.display().to_string())
                .field("records", report.records)
                .field("bytes", report.bytes),
        );
        Ok(report)
    }

    /// Re-establishes the records of an archive through the same validation, caps and byte
    /// budget as a write. A line that is refused is reported and skipped; restored records
    /// count as read now (so they are hot) and keep their dumped read counts.
    ///
    /// # Errors
    /// `bytes_cap` when the restored graph would pass its budget (nothing is restored); a store
    /// failure when the file cannot be read.
    pub fn restore(&self, file: &Path) -> Result<RestoreReport, GraphError> {
        let text = read_dump(file)?;
        let _guard = self.lock()?;
        let mut state = self.load()?;
        let mut access: AccessFile = read_json(&self.access_path())?;
        let mut counters: Counters = read_json(&self.counters_path())?;
        let res = self.restore_locked(&text, &mut state, &mut access, &mut counters);
        if let Err(e @ GraphError::Refused(..)) = &res {
            self.refuse_op("restore", &mut counters, e);
        }
        let bytes = serde_json::to_vec(&counters).map_err(|e| GraphError::Store(e.to_string()))?;
        write_atomic(&self.counters_path(), &bytes)?;
        let report = res?;
        self.observer.event(
            &Event::new("memory", Level::Info, names::MEMORY_RESTORED)
                .field("file", file.display().to_string())
                .field("restored", report.restored)
                .field("unchanged", report.unchanged)
                .field("refused", report.refused.len() as u64),
        );
        Ok(report)
    }

    fn restore_locked(
        &self,
        text: &str,
        state: &mut State,
        access: &mut AccessFile,
        counters: &mut Counters,
    ) -> Result<RestoreReport, GraphError> {
        let cfg = self.cfg;
        let now = self.now_secs();
        let mut report = RestoreReport::default();
        let (entities, links) = parse_dump(text, counters, &mut report);
        // Entities first, so a relationship in the same file finds its ends.
        for (n, e, count) in entities {
            let input = EntityInput {
                kind: e.kind,
                name: e.name,
                attributes: e.attributes,
                provenance: e.provenance,
            };
            match upsert_entity(state, &cfg, input) {
                Ok(put) => settle(&mut report, access, &put.record.id, put.written, count, now),
                Err(GraphError::Refused(r, why)) => refuse_line(&mut report, counters, n, r, why),
                Err(e) => return Err(e),
            }
        }
        for (n, r, count) in links {
            let input = RelationshipInput {
                from: r.from,
                to: r.to,
                kind: r.kind,
                attributes: r.attributes,
                provenance: r.provenance,
            };
            match upsert_relationship(state, &cfg, input) {
                Ok(put) => settle(&mut report, access, &put.record.id, put.written, count, now),
                Err(GraphError::Refused(r, why)) => refuse_line(&mut report, counters, n, r, why),
                Err(e) => return Err(e),
            }
        }
        check_budget(state, &cfg)?;
        if report.restored > 0 {
            let g = serde_json::to_vec(&*state).map_err(|e| GraphError::Store(e.to_string()))?;
            write_atomic(&self.graph_path(), &g)?;
            let a = serde_json::to_vec(&*access).map_err(|e| GraphError::Store(e.to_string()))?;
            write_atomic(&self.access_path(), &a)?;
            bump(&mut counters.archive_ops, "restored");
            counters.restored_records = counters.restored_records.saturating_add(report.restored);
        }
        Ok(report)
    }
}

/// Writes `content` to `path` (which must not exist) via a temp file, then re-reads it and
/// checks size and line count. A dump that fails any step is removed (it is ours) and refused.
fn write_verified(path: &Path, content: &str, records: u64) -> Result<(), GraphError> {
    if path.symlink_metadata().is_ok() {
        return Err(failed(format!("{} already exists", path.display())));
    }
    let tmp = path.with_extension("jsonl.tmp");
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(content.as_bytes())?;
            f.sync_all()
        });
    if let Err(e) = written {
        return Err(failed(format!("cannot write {}: {e}", tmp.display())));
    }
    let verified = fs::read(&tmp).map_or_else(
            |e| Err(format!("cannot re-read the dump: {e}")),
            |back| {
                let lines = String::from_utf8_lossy(&back).lines().count() as u64;
                if back.len() != content.len() || lines != records || back != content.as_bytes() {
                    Err(format!(
                        "verification failed: {} bytes / {lines} lines on disk, expected {} / {records}",
                        back.len(),
                        content.len()
                    ))
                } else {
                    Ok(())
                }
            },
        );
    if let Err(why) = verified {
        let _ = fs::remove_file(&tmp);
        return Err(failed(why));
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(failed(format!("cannot place {}: {e}", path.display())));
    }
    Ok(())
}

/// Parsed dump lines: `(line, record, dumped read count)`, entities and relationships apart.
type Parsed = (Vec<(usize, Entity, u64)>, Vec<(usize, Relationship, u64)>);

/// Parses the dump into entities and relationships (with their dumped read counts); a line that is
/// not a valid record, or whose id does not match its content, is refused and listed.
fn parse_dump(text: &str, counters: &mut Counters, report: &mut RestoreReport) -> Parsed {
    let mut entities: Vec<(usize, Entity, u64)> = Vec::new();
    let mut links: Vec<(usize, Relationship, u64)> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(raw) else {
            refuse_line(
                report,
                counters,
                n,
                Reason::Invalid,
                "not a dump line".into(),
            );
            continue;
        };
        let count = line.access.map_or(0, |a| a.access_count);
        match line.kind.as_str() {
            "entity" => match serde_json::from_value::<Entity>(line.record) {
                Ok(e) if e.id == entity_id(&e.kind, &e.name) => entities.push((n, e, count)),
                Ok(_) => refuse_line(
                    report,
                    counters,
                    n,
                    Reason::Invalid,
                    "id does not match kind and name".into(),
                ),
                Err(e) => refuse_line(report, counters, n, Reason::Invalid, e.to_string()),
            },
            "relationship" => match serde_json::from_value::<Relationship>(line.record) {
                Ok(r) if r.id == relationship_id(&r.from, &r.to, &r.kind) => {
                    links.push((n, r, count));
                }
                Ok(_) => refuse_line(
                    report,
                    counters,
                    n,
                    Reason::Invalid,
                    "id does not match from, to and kind".into(),
                ),
                Err(e) => refuse_line(report, counters, n, Reason::Invalid, e.to_string()),
            },
            other => refuse_line(
                report,
                counters,
                n,
                Reason::Invalid,
                format!("unknown type '{other}'"),
            ),
        }
    }
    (entities, links)
}

fn refuse_line(
    report: &mut RestoreReport,
    counters: &mut Counters,
    n: usize,
    r: Reason,
    why: String,
) {
    bump(&mut counters.write_refusals, r.as_str());
    report.refused.push((n, r.as_str().to_owned(), why));
}

fn settle(
    report: &mut RestoreReport,
    access: &mut AccessFile,
    id: &str,
    w: Written,
    count: u64,
    now: u64,
) {
    match w {
        Written::Unchanged => report.unchanged += 1,
        Written::Created | Written::Merged => {
            report.restored += 1;
            access.entries.entry(id.to_owned()).or_insert(Access {
                last_access_secs: now,
                access_count: count,
                observed: Some(Tier::Hot),
            });
        }
    }
}

fn read_dump(path: &Path) -> Result<String, GraphError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(failed(format!("no such archive {}", path.display())));
        }
        Err(e) => return Err(io_err("stat", path, e)),
    };
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(io_err(
            "read",
            path,
            "not a regular file within the size bound",
        ));
    }
    let mut buf = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES).read_to_end(&mut buf))
        .map_err(|e| io_err("read", path, e))?;
    String::from_utf8(buf).map_err(|e| io_err("read", path, e))
}
