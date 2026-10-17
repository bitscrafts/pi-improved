//! Spec 173: the autonomy loop's lifecycle lines as events, readable beside the daemon's own.
//!
//! The loop appends one JSON line per event to a file; the daemon's runtime log is a JSON-lines file
//! of `kind` + `ts_ms`. [`merged`] reads both and returns the newest `n` in time order, so one
//! reader covers both. Nothing here talks to a daemon or blocks on one.

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One loop event as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopEvent {
    /// Unix milliseconds.
    pub ts_ms: i64,
    /// The cycle id (`20261017T182226Z` style).
    pub cycle: String,
    /// `loop.cycle.start`, `loop.dispatch`, `loop.verify`, `loop.gate.verdict`, `loop.park`, `loop.commit`.
    pub kind: String,
    /// Free text: the session id, the failing step, the revision, a daemon-down note.
    pub detail: String,
}

/// One line of the merged view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Unix milliseconds.
    pub ts_ms: i64,
    /// `loop` or `daemon`.
    pub source: String,
    /// The event kind.
    pub kind: String,
    /// The cycle and detail for a loop event; the run id for a daemon one.
    pub detail: String,
}

/// Appends `ev` to `file`, creating it (and its directory) when absent.
///
/// # Errors
/// Fails if the file cannot be created or written.
pub fn append(file: &Path, ev: &LoopEvent) -> std::io::Result<()> {
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(ev).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    f.write_all(line.as_bytes())
}

fn loop_lines(file: &Path) -> Vec<Line> {
    let raw = std::fs::read_to_string(file).unwrap_or_default();
    raw.lines()
        .filter_map(|l| serde_json::from_str::<LoopEvent>(l).ok())
        .map(|e| Line {
            ts_ms: e.ts_ms,
            source: "loop".to_owned(),
            kind: e.kind,
            detail: if e.detail.is_empty() {
                format!("cycle={}", e.cycle)
            } else {
                format!("cycle={} {}", e.cycle, e.detail)
            },
        })
        .collect()
}

fn daemon_lines(log: &Path) -> Vec<Line> {
    let raw = std::fs::read_to_string(log).unwrap_or_default();
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            Some(Line {
                ts_ms: v.get("ts_ms")?.as_i64()?,
                source: "daemon".to_owned(),
                kind: v.get("kind")?.as_str()?.to_owned(),
                detail: v
                    .get("run")
                    .and_then(serde_json::Value::as_str)
                    .map(|r| format!("run={r}"))
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// The newest `n` events of the loop file and (when given) the daemon log, oldest first.
/// A missing or unreadable file contributes nothing.
#[must_use]
pub fn merged(file: &Path, daemon_log: Option<&Path>, n: usize) -> Vec<Line> {
    let mut all = loop_lines(file);
    if let Some(log) = daemon_log {
        all.extend(daemon_lines(log));
    }
    all.sort_by_key(|l| l.ts_ms);
    let skip = all.len().saturating_sub(n);
    all.split_off(skip)
}

/// One printable line: `<ts_ms> <source> <kind> <detail>`.
#[must_use]
pub fn render(l: &Line) -> String {
    format!("{} {} {} {}", l.ts_ms, l.source, l.kind, l.detail)
        .trim_end()
        .to_owned()
}
