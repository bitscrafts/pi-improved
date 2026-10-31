//! Spec 156: incidents from the daemon's own runtime log.
//!
//! The log is a JSON-lines file of events with `kind`, optional `run`, `ts_ms`.
//! A second source beside the
//! builder-loop reader of spec 150, selected explicitly; no model, and the log
//! is only ever read: the file is read whole into memory, nothing is written.
//!
//! Five anomalies open an incident: an unexpected restart (`daemon.started`
//! with no `daemon.stopped` since the last one), a start that did not reach
//! ready (a start marker not followed by `daemon.started`), a run with no
//! terminal event whose last record is older than the window at the log's end,
//! one refusal repeated `threshold` times inside the window, and one cron
//! firing repeated inside the window (spec 193).

use std::collections::BTreeMap;
use std::path::Path;

use crate::incident::IncidentConfig;

/// Event that marks a start attempt (emitted before the daemon is ready).
const START_MARK: &str = pii_observe::names::DAEMON_SANDBOX_IN_FORCE;
/// Spec 322: how long a start mark may wait for `daemon.started` before the scan calls the start lost.
/// A scan that runs while a healthy daemon is booting must stay silent; ten minutes of silence after a
/// mark is a start that never served.
const START_MARK_GRACE_SECS: i64 = 60;
/// Kinds whose repetition is a refusal incident.
const REFUSALS: [&str; 3] = [
    pii_observe::names::RUN_START_REFUSED,
    pii_observe::names::WORKER_REQUEST_REFUSED,
    pii_observe::names::CHANNEL_POLL_FAILED,
];

/// One incident opened from the runtime log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIncident {
    /// What happened, e.g. `run-stalled:<run id>`.
    pub fingerprint: String,
    /// Occurrences folded into this incident.
    pub count: u64,
    /// Unix seconds of the first event of the evidence.
    pub first_seen: i64,
    /// Unix seconds of the last event of the evidence.
    pub last_seen: i64,
    /// Unix seconds at which the condition became detectable.
    pub opened_at: i64,
    /// The log file it was read from.
    pub source: String,
}

impl RuntimeIncident {
    /// Detection latency: first event to incident, in seconds.
    #[must_use]
    pub const fn latency_secs(&self) -> i64 {
        self.opened_at - self.first_seen
    }
}

struct Run {
    first: i64,
    last: i64,
    terminal: bool,
}

/// One start of the daemon as the log records it.
///
/// The rule (spec 160): every
/// `daemon.started` record is exactly one start record; its first evidence is
/// the start marker (`daemon.sandbox.in_force`) written before it, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRecord {
    /// Unix milliseconds of the start marker, when one precedes the start.
    pub first_evidence_ms: Option<i64>,
    /// Unix milliseconds of the `daemon.started` record.
    pub started_ms: i64,
}

impl StartRecord {
    /// Time-to-notice in milliseconds: first evidence line to the start record.
    #[must_use]
    pub fn time_to_notice_ms(&self) -> Option<i64> {
        self.first_evidence_ms.map(|f| self.started_ms - f)
    }
}

/// Everything one scan read: incidents, start records, files considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeScan {
    /// Incidents, ordered by when they became detectable.
    pub incidents: Vec<RuntimeIncident>,
    /// Start records in log order (rotated files first, oldest to newest).
    pub starts: Vec<StartRecord>,
    /// Files read: rotated siblings `<name>.<N>` (oldest first), then `log`.
    pub files: Vec<String>,
}

/// `log` preceded by its rotated siblings `<name>.<N>`, oldest (highest N) first.
fn with_rotated(log: &Path) -> Vec<std::path::PathBuf> {
    let (Some(dir), Some(name)) = (log.parent(), log.file_name().and_then(|n| n.to_str())) else {
        return vec![log.to_path_buf()];
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let mut rotated: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e
                .file_name()
                .to_str()?
                .strip_prefix(name)?
                .strip_prefix('.')?
                .parse::<u64>()
                .ok()?;
            Some((n, e.path()))
        })
        .collect();
    rotated.sort_by_key(|b| std::cmp::Reverse(b.0));
    let mut out: Vec<_> = rotated.into_iter().map(|(_, p)| p).collect();
    out.push(log.to_path_buf());
    out
}

/// Incidents in the runtime log at `log`.
///
/// # Errors
/// Fails if the log cannot be read.
pub fn scan_runtime_log(log: &Path, cfg: &IncidentConfig) -> std::io::Result<Vec<RuntimeIncident>> {
    scan_runtime(log, cfg).map(|s| s.incidents)
}

/// Incidents, start records and files of the runtime log at `log` and its
/// rotated siblings (`--log-keep-files`): rotation is not a blind spot.
///
/// A start marker with no `daemon.started` after it is a failed start only in a
/// log that carries the daemon's lifecycle records (`daemon.started` or
/// `daemon.stopped`): the audit log holds start markers alone, so there the
/// absence of a readiness record is no evidence of anything.
///
/// # Errors
/// Fails if a file cannot be read.
#[allow(clippy::too_many_lines)] // one pass over the log; splitting it scatters the state
pub fn scan_runtime(log: &Path, cfg: &IncidentConfig) -> std::io::Result<RuntimeScan> {
    let paths = with_rotated(log);
    let mut text = String::new();
    for p in &paths {
        text.push_str(&String::from_utf8_lossy(&std::fs::read(p)?));
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    let files = paths.iter().map(|p| p.display().to_string()).collect();
    let source = log.display().to_string();
    let mk = |fp: String, count: u64, first: i64, last: i64, opened: i64| RuntimeIncident {
        fingerprint: fp,
        count,
        first_seen: first,
        last_seen: last,
        opened_at: opened,
        source: source.clone(),
    };
    let mut out = Vec::new();
    let mut runs: BTreeMap<String, Run> = BTreeMap::new();
    let mut refusals: BTreeMap<&'static str, Vec<i64>> = BTreeMap::new();
    let mut firings: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut log_end = i64::MIN;
    let mut starting: Option<i64> = None;
    let mut running: Option<i64> = None;
    let mut starts = Vec::new();
    let mut mark_ms: Option<i64> = None;
    let mut lifecycle = false;
    for line in text.lines() {
        let Some((kind, ms, run)) = parse(line) else {
            continue;
        };
        let ts = ms / 1000;
        log_end = log_end.max(ts);
        if kind == START_MARK || kind == pii_observe::names::DAEMON_STARTING {
            mark_ms = Some(ms);
            if let Some(t) = starting.replace(ts) {
                out.push(mk("daemon-start-not-ready".to_owned(), 1, t, t, ts));
            }
        } else if kind == pii_observe::names::DAEMON_STARTED {
            lifecycle = true;
            starts.push(StartRecord {
                first_evidence_ms: mark_ms.take(),
                started_ms: ms,
            });
            starting = None;
            if let Some(t) = running.replace(ts) {
                out.push(mk("daemon-restart-unexpected".to_owned(), 1, t, ts, ts));
            }
        } else if kind == pii_observe::names::DAEMON_STOPPED {
            lifecycle = true;
            mark_ms = None;
            running = None;
            starting = None;
        } else if let Some(r) = REFUSALS.iter().find(|r| **r == kind) {
            refusals.entry(r).or_default().push(ts);
        } else if kind == pii_observe::names::CRON_FIRED
            && let Some(cron) = cron_of(line)
        {
            firings.entry(cron).or_default().push(ts);
        }
        if let Some(id) = run {
            let r = runs.entry(id).or_insert(Run {
                first: ts,
                last: ts,
                terminal: false,
            });
            r.first = r.first.min(ts);
            r.last = r.last.max(ts);
            r.terminal |= kind == pii_observe::names::RUN_OUTCOME
                || kind == pii_observe::names::RUN_START_REFUSED;
        }
    }
    if let Some(t) = starting
        && log_end - t >= START_MARK_GRACE_SECS
    {
        out.push(mk("daemon-start-not-ready".to_owned(), 1, t, t, log_end));
    }
    if !lifecycle {
        out.retain(|i| i.fingerprint != "daemon-start-not-ready");
    }
    out.extend(
        runs.into_iter()
            .filter(|(_, r)| !r.terminal && r.last + cfg.window_secs <= log_end)
            .map(|(id, r)| {
                mk(
                    format!("run-stalled:{id}"),
                    1,
                    r.first,
                    r.last,
                    r.last + cfg.window_secs,
                )
            }),
    );
    for (kind, ts) in refusals {
        for (first, last, n, opened) in clusters(ts, cfg) {
            out.push(mk(
                format!("refusal-repeated:{kind}"),
                n,
                first,
                last,
                opened,
            ));
        }
    }
    // Spec 193: the assertion is `runs_per_occurrence == 1`, so a second firing in a window is the defect.
    let twice = IncidentConfig {
        threshold: 2,
        ..cfg.clone()
    };
    for (cron, ts) in firings {
        for (first, last, n, opened) in clusters(ts, &twice) {
            out.push(mk(
                format!("cron-occurrence-multiplied:{cron}"),
                n,
                first,
                last,
                opened,
            ));
        }
    }
    out.sort_by(|a, b| (a.opened_at, &a.fingerprint).cmp(&(b.opened_at, &b.fingerprint)));
    Ok(RuntimeScan {
        incidents: out,
        starts,
        files,
    })
}

/// Bursts of at least `threshold` timestamps, each within the window of the
/// previous: `(first, last, count, time the threshold was reached)`.
fn clusters(mut ts: Vec<i64>, cfg: &IncidentConfig) -> Vec<(i64, i64, u64, i64)> {
    let need = usize::try_from(cfg.threshold.max(1)).unwrap_or(usize::MAX);
    ts.sort_unstable();
    let mut out = Vec::new();
    let mut cluster: Vec<i64> = Vec::new();
    for t in ts.into_iter().map(Some).chain([None]) {
        if let Some(t) = t
            && cluster.last().is_none_or(|l| t - l <= cfg.window_secs)
        {
            cluster.push(t);
            continue;
        }
        if let (Some(&first), Some(&last)) = (cluster.first(), cluster.last())
            && cluster.len() >= need
        {
            let opened = cluster.get(need - 1).copied().unwrap_or(last);
            out.push((first, last, cluster.len() as u64, opened));
        }
        cluster.clear();
        cluster.extend(t);
    }
    out
}

/// The `cron` field of one log line, if it names one.
fn cron_of(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let f = v.get("fields")?.get("cron")?.as_str()?;
    Some(f.to_owned())
}

/// `(kind, unix milliseconds, run id)` of one log line; `None` if it is not an event.
fn parse(line: &str) -> Option<(String, i64, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let kind = v.get("kind")?.as_str()?.to_owned();
    let ts = v
        .get("ts_ms")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let run = v.get("run").and_then(|r| r.as_str()).map(str::to_owned);
    Some((kind, ts, run))
}
