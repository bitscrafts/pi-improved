//! Spec 156: incidents from the daemon's own runtime log.
//!
//! The log is a JSON-lines file of events with `kind`, optional `run`, `ts_ms`.
//! A second source beside the
//! builder-loop reader of spec 150, selected explicitly; no model, and the log
//! is only ever read: the file is read whole into memory, nothing is written.
//!
//! Four anomalies open an incident: an unexpected restart (`daemon.started`
//! with no `daemon.stopped` since the last one), a start that did not reach
//! ready (a start marker not followed by `daemon.started`), a run with no
//! terminal event whose last record is older than the window at the log's end,
//! and one refusal repeated `threshold` times inside the window.

use std::collections::BTreeMap;
use std::path::Path;

use crate::incident::IncidentConfig;

/// Event that marks a start attempt (emitted before the daemon is ready).
const START_MARK: &str = pii_observe::names::DAEMON_SANDBOX_IN_FORCE;
/// Kinds whose repetition is a refusal incident.
const REFUSALS: [&str; 2] = [
    pii_observe::names::RUN_START_REFUSED,
    pii_observe::names::WORKER_REQUEST_REFUSED,
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

/// Incidents in the runtime log at `log`.
///
/// # Errors
/// Fails if the log cannot be read.
pub fn scan_runtime_log(log: &Path, cfg: &IncidentConfig) -> std::io::Result<Vec<RuntimeIncident>> {
    let text = String::from_utf8_lossy(&std::fs::read(log)?).into_owned();
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
    let mut log_end = i64::MIN;
    let mut starting: Option<i64> = None;
    let mut running: Option<i64> = None;
    for line in text.lines() {
        let Some((kind, ts, run)) = parse(line) else {
            continue;
        };
        log_end = log_end.max(ts);
        if kind == START_MARK {
            if let Some(t) = starting.replace(ts) {
                out.push(mk("daemon-start-not-ready".to_owned(), 1, t, t, ts));
            }
        } else if kind == pii_observe::names::DAEMON_STARTED {
            starting = None;
            if let Some(t) = running.replace(ts) {
                out.push(mk("daemon-restart-unexpected".to_owned(), 1, t, ts, ts));
            }
        } else if kind == pii_observe::names::DAEMON_STOPPED {
            running = None;
            starting = None;
        } else if let Some(r) = REFUSALS.iter().find(|r| **r == kind) {
            refusals.entry(r).or_default().push(ts);
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
    if let Some(t) = starting {
        out.push(mk("daemon-start-not-ready".to_owned(), 1, t, t, log_end));
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
    out.sort_by(|a, b| (a.opened_at, &a.fingerprint).cmp(&(b.opened_at, &b.fingerprint)));
    Ok(out)
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

/// `(kind, unix seconds, run id)` of one log line; `None` if it is not an event.
fn parse(line: &str) -> Option<(String, i64, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let kind = v.get("kind")?.as_str()?.to_owned();
    let ts = v
        .get("ts_ms")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        / 1000;
    let run = v.get("run").and_then(|r| r.as_str()).map(str::to_owned);
    Some((kind, ts, run))
}
