//! Spec 150: an incident is opened from the system's own records, with no model.
//!
//! Reads the loop's `OUTCOME=` lines, the gate's failing-test names and
//! the daemon's own events as rows `(ts, source, fingerprint, path:line)`;
//! `N` rows of one fingerprint within the window open ONE incident, and later
//! rows of it only raise its count. It opens a record; it fixes nothing.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The append-only incident record, inside the scanned root.
pub const INCIDENTS_FILE: &str = "incidents.jsonl";
/// Optional JSON override of [`IncidentConfig`], inside the scanned root.
pub const INCIDENT_CONFIG_FILE: &str = "incidents.config.json";

/// Threshold, window and the one list of known-benign fingerprints (R2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IncidentConfig {
    /// Occurrences of one fingerprint, within `window_secs`, that open an incident.
    pub threshold: u32,
    /// The window, in seconds.
    pub window_secs: i64,
    /// Fingerprints that are expected and never open an incident.
    pub benign: Vec<String>,
}

impl Default for IncidentConfig {
    fn default() -> Self {
        Self {
            threshold: 3,
            window_secs: 30 * 60,
            benign: ["window-closed", "done", "queue-empty"]
                .map(str::to_owned)
                .to_vec(),
        }
    }
}

impl IncidentConfig {
    /// The root's override file when present and valid, else the defaults.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        std::fs::read(root.join(INCIDENT_CONFIG_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}

/// One recorded occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Unix seconds.
    pub ts: i64,
    /// `cycle`, `gate` or `daemon`.
    pub source: &'static str,
    /// The exact-match string that identifies the failure.
    pub fingerprint: String,
    /// Where it was read.
    pub path: PathBuf,
    /// 1-based line in `path`.
    pub line: usize,
}

/// One opened incident (a line of `incidents.jsonl`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incident {
    /// What repeated.
    pub fingerprint: String,
    /// First occurrence of the cluster, `YYYY-MM-DDTHH:MM:SSZ`.
    pub first_seen: String,
    /// The occurrence that reached the threshold.
    pub opened_at: String,
    /// Occurrences so far (never a second incident).
    pub count: u64,
    /// `path:line` of the first occurrence.
    pub evidence: String,
}

impl Incident {
    /// The evidence split into its path and 1-based line.
    #[must_use]
    pub fn evidence_parts(&self) -> Option<(&str, usize)> {
        let (p, l) = self.evidence.rsplit_once(':')?;
        Some((p, l.parse().ok()?))
    }
}

// Howard Hinnant's civil-date algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for Unix seconds.
#[must_use]
pub fn iso(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86_400));
    let s = ts.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// `YYYY-MM-DDTHH:MM:SSZ` → Unix seconds (the inverse of [`iso`]).
#[must_use]
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b.get(4) != Some(&b'-')
        || b.get(7) != Some(&b'-')
        || b.get(10) != Some(&b'T')
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
        || b.get(19) != Some(&b'Z')
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    Some(
        days_from_civil(n(0..4)?, n(5..7)?, n(8..10)?) * 86_400
            + n(11..13)? * 3600
            + n(14..16)? * 60
            + n(17..19)?,
    )
}

/// `20261011T154626Z` → Unix seconds.
fn parse_compact(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 15 || b.get(8) != Some(&b'T') {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    Some(
        days_from_civil(n(0..4)?, n(4..6)?, n(6..8)?) * 86_400
            + n(9..11)? * 3600
            + n(11..13)? * 60
            + n(13..15)?,
    )
}

/// `[15:46:30Z] …` → seconds into the day.
fn leading_time(line: &str) -> Option<i64> {
    let t = line.strip_prefix('[')?.get(..9)?;
    if t.get(2..3) != Some(":") || t.get(5..6) != Some(":") || t.get(8..9) != Some("Z") {
        return None;
    }
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    Some(n(0..2)? * 3600 + n(3..5)? * 60 + n(6..8)?)
}

/// Rows from one cycle log. A line carries only a time of day; the date comes
/// from the `=== cycle <stamp>` header (or the file name), and a clock that
/// goes backwards rolls the date over.
fn cycle_rows(path: &Path, text: &str, out: &mut Vec<Row>) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut day = name
        .strip_prefix("cycle-")
        .and_then(parse_compact)
        .map_or(0, |t| t.div_euclid(86_400));
    let mut tod = 0;
    for (i, line) in text.lines().enumerate() {
        if let Some(stamp) = line.split("=== cycle ").nth(1).and_then(parse_compact) {
            day = stamp.div_euclid(86_400);
            tod = stamp.rem_euclid(86_400);
        } else if let Some(t) = leading_time(line) {
            if t < tod {
                day += 1;
            }
            tod = t;
        }
        if let Some(v) = line.strip_prefix("OUTCOME=") {
            let fingerprint = v.split_whitespace().next().unwrap_or("");
            if !fingerprint.is_empty() {
                out.push(Row {
                    ts: day * 86_400 + tod,
                    source: "cycle",
                    fingerprint: fingerprint.to_owned(),
                    path: path.to_path_buf(),
                    line: i + 1,
                });
            }
        }
    }
}

/// Rows from a `fresh-gate` output: one per `test <name> ... FAILED`.
fn gate_rows(path: &Path, text: &str, ts: i64, out: &mut Vec<Row>) {
    for (i, line) in text.lines().enumerate() {
        let Some(name) = line
            .strip_prefix("test ")
            .and_then(|r| r.strip_suffix(" ... FAILED"))
        else {
            continue;
        };
        out.push(Row {
            ts,
            source: "gate",
            fingerprint: name.trim().to_owned(),
            path: path.to_path_buf(),
            line: i + 1,
        });
    }
}

fn is_incident_kind(kind: &str) -> bool {
    kind == pii_observe::names::RUN_START_REFUSED
        || kind == pii_observe::names::NODE_PANICKED
        || (kind.starts_with("store.") && kind.rsplit('.').next() == Some("error"))
}

/// Rows from a daemon JSON-lines log (`kind`, `ts_ms`).
fn daemon_rows(path: &Path, text: &str, out: &mut Vec<Row>) {
    for (i, line) in text.lines().enumerate() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(kind) = v.get("kind").and_then(|k| k.as_str()) else {
            continue;
        };
        if !is_incident_kind(kind) {
            continue;
        }
        let ms = v
            .get("ts_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        out.push(Row {
            ts: ms / 1000,
            source: "daemon",
            fingerprint: kind.to_owned(),
            path: path.to_path_buf(),
            line: i + 1,
        });
    }
}

fn mtime(path: &Path) -> i64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Every row under `root`: `cycle-*.log`, `daemon*.log*`, and `gate-output.log`
/// in `root` or one directory below it. Unreadable files are skipped.
#[must_use]
pub fn read_rows(root: &Path) -> Vec<Row> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(e.path());
            }
        }
    }
    for d in &dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            files.extend(rd.flatten().map(|e| e.path()));
        }
    }
    files.sort();
    let mut rows = Vec::new();
    for p in files {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        let in_root = p.parent() == Some(root);
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        if in_root && name.starts_with("cycle-") && p.extension().is_some_and(|e| e == "log") {
            cycle_rows(&p, &text, &mut rows);
        } else if in_root && name.starts_with("daemon") && name.contains(".log") {
            daemon_rows(&p, &text, &mut rows);
        } else if name == "gate-output.log" {
            gate_rows(&p, &text, mtime(&p), &mut rows);
        }
    }
    rows
}

struct Open {
    last: i64,
}

/// The incidents the rows imply (R2, R3).
///
/// A fingerprint opens one incident at its `threshold`-th occurrence inside the window; further occurrences, each
/// within the window of the previous, raise that incident's count. A longer
/// silence ends it, so a later burst is a new incident.
#[must_use]
pub fn detect(rows: &[Row], cfg: &IncidentConfig) -> Vec<Incident> {
    let mut by_fp: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        if !cfg.benign.iter().any(|b| b == &r.fingerprint) {
            by_fp.entry(&r.fingerprint).or_default().push(r);
        }
    }
    let need = usize::try_from(cfg.threshold.max(1)).unwrap_or(usize::MAX);
    let mut out: Vec<Incident> = Vec::new();
    for (fp, mut rs) in by_fp {
        rs.sort_by_key(|r| r.ts);
        let mut open: Option<Open> = None;
        let mut pending: Vec<&Row> = Vec::new();
        for r in rs {
            if let Some(o) = &mut open {
                if r.ts - o.last <= cfg.window_secs {
                    o.last = r.ts;
                    if let Some(inc) = out.last_mut() {
                        inc.count += 1;
                    }
                    continue;
                }
                open = None;
            }
            pending.retain(|p| r.ts - p.ts <= cfg.window_secs);
            pending.push(r);
            if pending.len() >= need {
                let Some(&first) = pending.first() else {
                    continue;
                };
                out.push(Incident {
                    fingerprint: fp.to_owned(),
                    first_seen: iso(first.ts),
                    opened_at: iso(r.ts),
                    count: pending.len() as u64,
                    evidence: format!("{}:{}", first.path.display(), first.line),
                });
                open = Some(Open { last: r.ts });
                pending.clear();
            }
        }
    }
    out.sort_by(|a, b| (&a.opened_at, &a.fingerprint).cmp(&(&b.opened_at, &b.fingerprint)));
    out
}

/// Reads the records under `root` and detects incidents.
///
/// Reconciles them with
/// `<root>/incidents.jsonl`: an unknown incident is appended (and flagged
/// new), a known one only has its count refreshed. Returns every incident with
/// its newness, so a rescan of unchanged records reports nothing new.
///
/// # Errors
/// Fails if the incident file cannot be written.
pub fn scan_incidents(root: &Path, cfg: &IncidentConfig) -> std::io::Result<Vec<(Incident, bool)>> {
    let found = detect(&read_rows(root), cfg);
    let file = root.join(INCIDENTS_FILE);
    let mut known: Vec<Incident> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let mut result = Vec::new();
    let mut dirty = false;
    let mut appended = String::new();
    for inc in found {
        let at = known
            .iter()
            .position(|k| k.fingerprint == inc.fingerprint && k.first_seen == inc.first_seen);
        let is_new = at.is_none();
        match at {
            Some(i) => {
                if let Some(k) = known.get_mut(i).filter(|k| k.count != inc.count) {
                    k.count = inc.count;
                    dirty = true;
                }
            }
            None => {
                let _ = writeln!(appended, "{}", serde_json::to_string(&inc)?);
            }
        }
        result.push((inc, is_new));
    }
    if dirty {
        let mut text = String::new();
        for k in &known {
            let _ = writeln!(text, "{}", serde_json::to_string(k)?);
        }
        text.push_str(&appended);
        let tmp = root.join(format!("{INCIDENTS_FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &file)?;
    } else if !appended.is_empty() {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?
            .write_all(appended.as_bytes())?;
    }
    Ok(result)
}
