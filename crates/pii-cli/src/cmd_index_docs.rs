//! `pii index docs [--root <dir>] [--kind spec|handoff|ops|all]` (spec 205): feeds the implementation
//! record into pii's own memory. It derives one document per unit from `specs/*.md`, `HANDOFF.md`
//! (one per `## ` section) and `_docs/05-operations/*.md`, and writes each through `memory.put`.
//! Idempotency lives in the store: a document's id is the hash of its text, and the daemon reports
//! a put it already holds as `unchanged`, so a second pass over unchanged files writes nothing.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_proto::method::Method;

use crate::cmd_ctl::{CallError, next_str, run_call_typed};
use crate::cmd_index::clip;

const USAGE: &str = "usage: pii index docs [--root <dir>] [--kind spec|handoff|ops|all]";
/// Prose budget per document, in bytes.
const BODY_BYTES: usize = 600;
const PLAN: &str = "scripts/autonomy/plan.tsv";
const STATE: &str = "_tmp/autonomy/state.json";

/// Counts printed as `documents=<n> unchanged=<m> failed=<k>`.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    documents: usize,
    unchanged: usize,
    failed: usize,
}

impl Counts {
    fn absorb(&mut self, other: &Self) {
        self.documents += other.documents;
        self.unchanged += other.unchanged;
        self.failed += other.failed;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Spec,
    Handoff,
    Ops,
}

fn kinds_named(v: &str) -> Option<Vec<Kind>> {
    Some(match v {
        "spec" | "specs" => vec![Kind::Spec],
        "handoff" => vec![Kind::Handoff],
        "ops" => vec![Kind::Ops],
        "all" => vec![Kind::Spec, Kind::Handoff, Kind::Ops],
        _ => return None,
    })
}

/// `pii index docs ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_docs(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut root = PathBuf::from(".");
    let mut kinds = vec![Kind::Spec, Kind::Handoff, Kind::Ops];
    while let Some(a) = next_str(args) {
        let ok = match a.as_str() {
            "--root" => next_str(args).map(|v| root = PathBuf::from(v)).is_some(),
            "--kind" => next_str(args)
                .and_then(|v| kinds_named(&v))
                .map(|k| kinds = k)
                .is_some(),
            _ => false,
        };
        if !ok {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            return ExitCode::from(2);
        }
    }
    let queue = Queue::load(&root);
    // `Ok(true)`: the store took a new document; `Ok(false)`: it already held the same text.
    let mut put = |doc: &str| -> Result<bool, String> {
        let first = doc.lines().next().unwrap_or("");
        let v = run_call_typed(Method::MemoryPut, serde_json::json!({"text": doc}))
            .map_err(CallError::into_message)
            .map_err(|e| format!("{first}: {e}"))?;
        Ok(v.get("unchanged") != Some(&serde_json::json!(true)))
    };
    let mut total = Counts::default();
    for kind in kinds {
        match sources(&root, kind) {
            Ok(files) => {
                let units = files.into_iter().flat_map(|p| docs_of(kind, &p, &queue));
                let counts = ingest(units, &mut put);
                total.absorb(&counts);
            }
            Err(e) => {
                total.failed += 1;
                let _ = writeln!(std::io::stderr(), "FAIL {e}");
            }
        }
    }
    let _ = writeln!(
        std::io::stdout(),
        "documents={} unchanged={} failed={}",
        total.documents,
        total.unchanged,
        total.failed
    );
    if total.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// The files one kind reads, in name order. A missing `specs/` is an error; a missing
/// `HANDOFF.md` or operations directory simply has no units.
fn sources(root: &Path, kind: Kind) -> Result<Vec<PathBuf>, String> {
    match kind {
        Kind::Spec => md_files(&root.join("specs")),
        Kind::Handoff => {
            let p = root.join("HANDOFF.md");
            Ok(if p.is_file() { vec![p] } else { Vec::new() })
        }
        Kind::Ops => {
            let dir = root.join("_docs/05-operations");
            if dir.is_dir() {
                md_files(&dir)
            } else {
                Ok(Vec::new())
            }
        }
    }
}

/// Top-level `*.md` files of `dir`, sorted. Subdirectories (archives) are not read.
fn md_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md"))
        .collect();
    v.sort();
    Ok(v)
}

/// The documents one file yields. Read errors are units that failed, not silent skips.
fn docs_of(kind: Kind, path: &Path, queue: &Queue) -> Vec<Result<String, String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return vec![Err(format!("{}: {e}", path.display()))],
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match kind {
        Kind::Spec => {
            let (row, status) = queue.status_of(name);
            vec![Ok(spec_doc(name, &text, &row, &status))]
        }
        Kind::Handoff => handoff_docs(&text).into_iter().map(Ok).collect(),
        Kind::Ops => vec![Ok(ops_doc(name, &text))],
    }
}

/// Collapses runs of whitespace to single spaces.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `## Overview` section of a spec, collapsed and clipped.
fn overview(text: &str) -> String {
    let rest = text.split_once("## Overview").map_or("", |(_, r)| r);
    let section = rest.split("\n## ").next().unwrap_or("");
    clip(&collapse(section), BODY_BYTES)
}

/// One spec: number, slug, title, queue status and row, every exit criterion verbatim, every
/// guardrail, and the overview.
fn spec_doc(file: &str, text: &str, row: &str, status: &str) -> String {
    let stem = file.strip_suffix(".md").unwrap_or(file);
    let (num, slug) = match stem.split_once('-') {
        Some((n, s)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (n, s),
        _ => ("?", stem),
    };
    let title = text
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(stem);
    let criteria: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix("- [ ] ")
                .or_else(|| l.strip_prefix("- [x] "))
                .or_else(|| l.strip_prefix("- [X] "))
        })
        .collect();
    let guardrails: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- Do NOT"))
        .collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "PII-SPEC {num} | status={status} | row={row} | {title}"
    );
    let _ = writeln!(
        out,
        "file=specs/{file} slug={slug} criteria={}",
        criteria.len()
    );
    for c in criteria {
        let _ = writeln!(out, "criterion: {c}");
    }
    for g in guardrails {
        let _ = writeln!(out, "guardrail: {g}");
    }
    let _ = write!(out, "body: {}", overview(text));
    out
}

/// One document per `## ` section of HANDOFF.md; the preamble and empty sections are not units.
fn handoff_docs(text: &str) -> Vec<String> {
    let padded = format!("\n{text}");
    padded
        .split("\n## ")
        .skip(1)
        .filter_map(|part| {
            let (head, rest) = part.split_once('\n').unwrap_or((part, ""));
            let body = clip(&collapse(rest), BODY_BYTES);
            (!body.is_empty()).then(|| format!("PII-HANDOFF | {}\nbody: {body}", head.trim()))
        })
        .collect()
}

/// One operational doc: its title, its file, and the head of its prose.
fn ops_doc(file: &str, text: &str) -> String {
    let stem = file.strip_suffix(".md").unwrap_or(file);
    let head = text
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(stem);
    let body = clip(&collapse(text), BODY_BYTES);
    format!("PII-OPS | {head}\nfile=_docs/05-operations/{file}\nbody: {body}")
}

/// Queue rows from `scripts/autonomy/plan.tsv` and the queue state from `_tmp/autonomy/state.json`.
struct Queue {
    /// `(row id, spec file name)` for each plan row.
    rows: Vec<(String, String)>,
    state: serde_json::Value,
}

impl Queue {
    fn load(root: &Path) -> Self {
        let rows = std::fs::read_to_string(root.join(PLAN))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with('A'))
            .filter_map(|l| {
                let mut f = l.split('\t');
                Some((f.next()?.to_owned(), f.next()?.to_owned()))
            })
            .collect();
        let state = std::fs::read_to_string(root.join(STATE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null);
        Self { rows, state }
    }

    /// `(row id, status)` for a spec file; `("-", "no-row")` when no queue row names it.
    fn status_of(&self, spec: &str) -> (String, String) {
        let Some((row, _)) = self.rows.iter().find(|(_, s)| s == spec) else {
            return (String::from("-"), String::from("no-row"));
        };
        let status = self
            .state
            .get(row)
            .and_then(|v| v.get("status"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("pending");
        (row.clone(), status.to_owned())
    }
}

/// Writes each unit through `put`, streaming: one document is built, put, and dropped at a time.
fn ingest(
    units: impl Iterator<Item = Result<String, String>>,
    put: &mut impl FnMut(&str) -> Result<bool, String>,
) -> Counts {
    let mut c = Counts::default();
    for unit in units {
        match unit.and_then(|doc| put(&doc)) {
            Ok(true) => c.documents += 1,
            Ok(false) => c.unchanged += 1,
            Err(e) => {
                c.failed += 1;
                let _ = writeln!(std::io::stderr(), "FAIL {e}");
            }
        }
    }
    c
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    const SPEC: &str = "# Spec: the record is ingested\n\n## Overview\nA verb, not a script.\n\n## Requirements\n- x\n\n## Exit Criteria\n- [ ] `pii index docs` exits 0\n- [x] second one\n\n## Guardrails\n- Do NOT key on a file.\n- Do NOT ingest _tmp.\n";

    #[test]
    fn a_spec_document_carries_number_status_row_criteria_and_guardrails() {
        let doc = spec_doc("205-the-record.md", SPEC, "A205", "pending");
        assert!(
            doc.starts_with(
                "PII-SPEC 205 | status=pending | row=A205 | Spec: the record is ingested\n"
            ),
            "{doc}"
        );
        assert!(
            doc.contains("file=specs/205-the-record.md slug=the-record criteria=2\n"),
            "{doc}"
        );
        assert!(
            doc.contains("criterion: `pii index docs` exits 0\n"),
            "{doc}"
        );
        assert!(doc.contains("criterion: second one\n"), "{doc}");
        assert!(
            doc.contains("guardrail: - Do NOT key on a file.\n"),
            "{doc}"
        );
        assert!(doc.ends_with("body: A verb, not a script."), "{doc}");
    }

    #[test]
    fn a_spec_without_a_queue_row_is_no_row_never_pending() {
        let doc = spec_doc("196-old.md", SPEC, "-", "no-row");
        assert!(
            doc.starts_with("PII-SPEC 196 | status=no-row | row=- | "),
            "{doc}"
        );
        assert!(!doc.contains("pending"), "{doc}");
    }

    #[test]
    fn a_queue_row_takes_its_state_and_a_missing_row_is_no_row() {
        let d = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "index-docs-queue-")
            .unwrap();
        std::fs::create_dir_all(d.path().join("scripts/autonomy")).unwrap();
        std::fs::create_dir_all(d.path().join("_tmp/autonomy")).unwrap();
        std::fs::write(
            d.path().join("scripts/autonomy/plan.tsv"),
            "# comment\nA7\t196-x.md\t-\ttitle\tv\tp\t1\n",
        )
        .unwrap();
        std::fs::write(
            d.path().join("_tmp/autonomy/state.json"),
            r#"{"A7":{"status":"done"}}"#,
        )
        .unwrap();
        let q = Queue::load(d.path());
        assert_eq!(
            q.status_of("196-x.md"),
            (String::from("A7"), String::from("done"))
        );
        assert_eq!(
            q.status_of("999-none.md"),
            (String::from("-"), String::from("no-row"))
        );
    }

    #[test]
    fn handoff_is_one_document_per_section_and_skips_the_preamble_and_empty_sections() {
        let docs =
            handoff_docs("preamble\n## Alpha\nfirst body\nmore\n## Beta\n\n## Gamma\nthird\n");
        assert_eq!(docs.len(), 2, "{docs:?}");
        assert_eq!(docs[0], "PII-HANDOFF | Alpha\nbody: first body more");
        assert_eq!(docs[1], "PII-HANDOFF | Gamma\nbody: third");
    }

    #[test]
    fn an_ops_document_is_headed_by_its_title_and_names_its_file() {
        let doc = ops_doc("071-x.md", "# Path to production\n\nbody text\n");
        assert_eq!(
            doc,
            "PII-OPS | Path to production\nfile=_docs/05-operations/071-x.md\nbody: # Path to production body text"
        );
    }

    #[test]
    fn ops_reads_only_the_top_level_so_archived_docs_are_not_ingested() {
        let d =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "index-docs-ops-").unwrap();
        let ops = d.path().join("_docs/05-operations");
        std::fs::create_dir_all(ops.join("archive")).unwrap();
        std::fs::write(ops.join("a.md"), "# A\n").unwrap();
        std::fs::write(ops.join("archive/old.md"), "# Old\n").unwrap();
        let files = sources(d.path(), Kind::Ops).unwrap();
        assert_eq!(files, vec![ops.join("a.md")]);
    }

    #[test]
    fn ingest_counts_written_unchanged_and_failed_units() {
        let units = vec![
            Ok(String::from("a")),
            Ok(String::from("b")),
            Err(String::from("unreadable")),
            Ok(String::from("c")),
        ];
        let mut put = |doc: &str| -> Result<bool, String> {
            match doc {
                "a" => Ok(true),
                "b" => Ok(false),
                _ => Err(String::from("store down")),
            }
        };
        let c = ingest(units.into_iter(), &mut put);
        assert_eq!(
            c,
            Counts {
                documents: 1,
                unchanged: 1,
                failed: 2
            }
        );
    }
}
