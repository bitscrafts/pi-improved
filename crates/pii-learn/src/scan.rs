//! The bounded, symlink-refusing reader (spec 123 R6). The substrate is DATA:
//! this module only ever extracts closed-vocabulary tokens and numbers from
//! it, never free text, so no trajectory string can reach an output.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::time::Instant;

use pii_observe::names;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::LearnConfig;

/// Events that mean "a limit or a refusal was hit" (specs 115, 120, 121, 122).
/// These are the registered names; there is no second taxonomy.
const LIMIT_KINDS: [&str; 7] = [
    names::TRAJECTORY_TRUNCATED,
    names::WASM_MEMORY_LIMIT,
    names::WASM_TIMEOUT,
    names::WASM_FUEL_EXHAUSTED,
    names::WASM_REFUSAL,
    names::WORKER_REQUEST_REFUSED,
    names::RUN_START_REFUSED,
];

/// `wasm.refusal`'s documented `reason` values; anything else is `other`.
const WASM_REASONS: [&str; 8] = [
    "not_configured",
    "out_of_scope",
    "unreadable",
    "too_large",
    "imports",
    "bad_module",
    "bad_abi",
    "invalid_output",
];

/// What one trajectory file says about its run, reduced to tokens and numbers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunSummary {
    /// The run id (the file stem).
    pub run: String,
    /// Path relative to the scan root, e.g. `trajectories/r1.jsonl`.
    pub path: String,
    /// The graph name, sanitised.
    pub graph: String,
    /// `success`, `failure`, `cancelled`, `suspended`, or `unknown` (no outcome).
    pub verdict: String,
    /// `(node, error class)` for each `NodeFailed`, in order; node `-` is the
    /// run terminal when no node failed.
    pub failures: Vec<(String, String)>,
    /// Limit/refusal event counts keyed `kind` or `kind:reason`.
    pub limits: BTreeMap<String, u64>,
    /// Sum of `NodeFinished.dur_ms`.
    pub dur_ms: u64,
    /// The file's size in bytes.
    pub bytes: u64,
}

/// What a scan read and what it set aside, with a reason for each.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanStats {
    /// Directory entries looked at.
    pub files_seen: u64,
    /// Files read and summarised.
    pub files_scanned: u64,
    /// Bytes read.
    pub bytes_read: u64,
    /// Lines that were not valid JSON (the rest of the file is still used).
    pub corrupt_lines: u64,
    /// Files skipped, by reason: `symlink`, `not_file`, `outside_root`,
    /// `too_large`, `unreadable`, `corrupt`, `over_file_cap`, `over_byte_cap`.
    pub skipped: BTreeMap<String, u64>,
    /// Wall-clock milliseconds the scan took.
    pub duration_ms: u64,
}

impl ScanStats {
    fn skip(&mut self, why: &str) {
        *self.skipped.entry(why.to_owned()).or_default() += 1;
    }
}

/// A scan root that cannot be read at all.
#[derive(Debug)]
pub struct ScanError(pub String);

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScanError {}

/// Keeps `[A-Za-z0-9._:/-]`, maps the rest to `_`, caps at 64 chars. A node or
/// graph name from a trajectory is data; this stops it carrying a sentence.
#[must_use]
pub fn token(s: &str) -> String {
    let t: String = s
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._:/-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if t.is_empty() { String::from("_") } else { t }
}

/// Maps an error text to one of a closed set of classes. The text itself is
/// never kept, so it cannot appear in a claim.
#[must_use]
pub fn classify(error: &str) -> &'static str {
    let e = error.to_ascii_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| e.contains(w));
    if has(&["timed out", "timeout", "deadline"]) {
        "timeout"
    } else if has(&["rate limit", "429"]) {
        "rate_limited"
    } else if has(&["out of memory", "memory limit"]) {
        "memory"
    } else if has(&["fuel"]) {
        "fuel"
    } else if has(&["denied", "permission", "forbidden", "refused"]) {
        "denied"
    } else if has(&["not found", "no such"]) {
        "not_found"
    } else if has(&["invalid", "parse", "malformed", "schema"]) {
        "invalid"
    } else if has(&["connection", "broken pipe", "i/o", "io error"]) {
        "io"
    } else {
        "other"
    }
}

fn verdict_token(v: &str) -> &'static str {
    match v {
        "success" => "success",
        "failure" => "failure",
        "cancelled" => "cancelled",
        "suspended" => "suspended",
        _ => "unknown",
    }
}

fn limit_key(kind: &str, fields: &Value) -> String {
    if kind != names::WASM_REFUSAL {
        return kind.to_owned();
    }
    let r = fields.get("reason").and_then(Value::as_str).unwrap_or("");
    let r = WASM_REASONS
        .iter()
        .find(|w| **w == r)
        .copied()
        .unwrap_or("other");
    format!("{kind}:{r}")
}

fn apply_line(s: &mut RunSummary, v: &Value) {
    let Some(kind) = v.get("kind").and_then(Value::as_str) else {
        return;
    };
    let fields = v.get("fields").unwrap_or(&Value::Null);
    if kind == names::RUN_OUTCOME {
        let verdict = fields.get("verdict").and_then(Value::as_str).unwrap_or("");
        verdict_token(verdict).clone_into(&mut s.verdict);
        // A run that failed without any node failing (a scheduler-level
        // failure) is recorded at the run terminal, node `-`.
        if s.verdict == "failure" && s.failures.is_empty() {
            let detail = fields.get("detail").and_then(Value::as_str).unwrap_or("");
            s.failures
                .push((String::from("-"), classify(detail).to_owned()));
        }
    } else if LIMIT_KINDS.contains(&kind) {
        *s.limits.entry(limit_key(kind, fields)).or_default() += 1;
    } else if kind == "run.journal" {
        apply_journal(s, fields.get("event").unwrap_or(&Value::Null));
    }
}

fn apply_journal(s: &mut RunSummary, ev: &Value) {
    if let Some(g) = ev.pointer("/Started/graph").and_then(Value::as_str) {
        s.graph = token(g);
    }
    if let Some(d) = ev.pointer("/NodeFinished/dur_ms").and_then(Value::as_u64) {
        s.dur_ms += d;
    }
    if let Some(f) = ev.get("NodeFailed") {
        let node = f.get("node").and_then(Value::as_str).unwrap_or("?");
        let err = f.get("error").and_then(Value::as_str).unwrap_or("");
        s.failures.push((token(node), classify(err).to_owned()));
    }
}

/// Summarises one file's text. Returns the summary and the count of corrupt
/// lines; `None` when no line parsed at all (the file is `corrupt`).
fn summarise(text: &str, run: &str, path: &str, bytes: u64) -> (Option<RunSummary>, u64) {
    let mut s = RunSummary {
        run: run.to_owned(),
        path: path.to_owned(),
        graph: String::from("?"),
        verdict: String::from("unknown"),
        bytes,
        ..RunSummary::default()
    };
    let (mut good, mut bad) = (0u64, 0u64);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v.is_object() => {
                good += 1;
                apply_line(&mut s, &v);
            }
            _ => bad += 1,
        }
    }
    ((good > 0).then_some(s), bad)
}

/// Reads at most `limit` bytes; `None` if the file is unreadable or not UTF-8.
fn read_bounded(path: &Path, limit: u64) -> Option<(String, u64)> {
    let mut buf = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut buf)
        .ok()?;
    let n = buf.len() as u64;
    Some((String::from_utf8(buf).ok()?, n))
}

/// Scans `<root>/trajectories/*.jsonl` in file-name order, inside `cfg`'s bounds.
///
/// A symlink (file or the directory itself) that leaves `root` is
/// refused, a corrupt file is skipped with a counted reason, nothing panics.
///
/// # Errors
/// [`ScanError`] when the root or its `trajectories` directory cannot be
/// resolved, or the directory resolves outside the root.
pub fn scan(root: &Path, cfg: &LearnConfig) -> Result<(Vec<RunSummary>, ScanStats), ScanError> {
    let started = Instant::now();
    let root_c = dunce::canonicalize(root)
        .map_err(|e| ScanError(format!("root {}: {e}", root.display())))?;
    let dir = dunce::canonicalize(root_c.join("trajectories"))
        .map_err(|e| ScanError(format!("{}/trajectories: {e}", root.display())))?;
    if !dir.starts_with(&root_c) {
        return Err(ScanError(String::from(
            "trajectories directory resolves outside the root",
        )));
    }
    let mut files: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| ScanError(format!("read_dir: {e}")))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| Path::new(n).extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    let mut stats = ScanStats::default();
    let mut runs = Vec::new();
    for name in files {
        stats.files_seen += 1;
        if usize::try_from(stats.files_scanned).is_ok_and(|n| n >= cfg.max_files) {
            stats.skip("over_file_cap");
            continue;
        }
        if stats.bytes_read >= cfg.max_total_bytes {
            stats.skip("over_byte_cap");
            continue;
        }
        let path = dir.join(&name);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            stats.skip("unreadable");
            continue;
        };
        if meta.file_type().is_symlink() {
            stats.skip("symlink");
        } else if !meta.is_file() {
            stats.skip("not_file");
        } else if meta.len() > cfg.max_file_bytes {
            stats.skip("too_large");
        } else if !dunce::canonicalize(&path).is_ok_and(|c| c.starts_with(&dir)) {
            stats.skip("outside_root");
        } else {
            read_one(&path, &name, cfg, &mut stats, &mut runs);
        }
    }
    stats.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok((runs, stats))
}

fn read_one(
    path: &Path,
    name: &str,
    cfg: &LearnConfig,
    stats: &mut ScanStats,
    runs: &mut Vec<RunSummary>,
) {
    let Some((text, n)) = read_bounded(path, cfg.max_file_bytes) else {
        stats.skip("unreadable");
        return;
    };
    stats.bytes_read += n;
    if n > cfg.max_file_bytes {
        stats.skip("too_large");
        return;
    }
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let rel = format!("trajectories/{name}");
    let (summary, bad) = summarise(&text, &token(stem), &rel, n);
    stats.corrupt_lines += bad;
    match summary {
        Some(s) => {
            stats.files_scanned += 1;
            runs.push(s);
        }
        None => stats.skip("corrupt"),
    }
}
