//! Spec 217 journal analysis: the trace lines and agent calls a run's journal shows. Built from the events
//! only, so a node's own returned value is never read here.

use std::path::Path;
use std::process::ExitCode;

use pii_abi::{NodeId, RunEvent};

use super::run::{read_via_daemon, read_via_store};

// --- Spec 217: `pii run trace` and `pii run explain`. Built from the run's journal events only: a node's returned
// value is never read here, so a node cannot report its own verdict into the trace.

/// One agent call a node made, as the journal recorded it: its spawn, and its exit once one landed.
#[derive(Debug, Clone)]
pub(super) struct AgentCall {
    agent: String,
    model: String,
    pid: i64,
    image_digest: String,
    exit: Option<CallExit>,
}

/// How an agent call ended (spec 217 R4): its code, its duration, and the criteria it ran.
#[derive(Debug, Clone)]
struct CallExit {
    rc: i64,
    dur_ms: i64,
    passed: i64,
    run: i64,
    first_failure: String,
    /// The head of the agent's stderr: the reason it exited, when it wrote one.
    stderr_head: String,
}

/// One trace line: a node that finished or failed, with the agent calls it made up to that point.
#[derive(Debug, Clone)]
pub(super) struct NodeLine {
    pub(super) seq: u64,
    pub(super) node: String,
    pub(super) kind: &'static str,
    start_ms: i64,
    dur_ms: Option<i64>,
    pub(super) calls: Vec<AgentCall>,
    /// The refusal the node journalled before any spawn, if it made one (spec 223 R2).
    refusal: Option<Refusal>,
    /// The `<fragment>#<node>` origin of a woven node (spec 219 R4).
    woven_from: Option<String>,
    /// The error the node failed with, when the journal shows it failing (spec 224 R2).
    pub(super) failure: Option<String>,
}

/// A `worker.start.refused` a node journalled: the scope's or the node's own reason, and the image digest slot.
#[derive(Debug, Clone)]
pub(super) struct Refusal {
    reason: String,
    image_digest: String,
}

pub(super) fn str_field(fields: &serde_json::Value, key: &str) -> String {
    fields
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn int_field(fields: &serde_json::Value, key: &str) -> i64 {
    fields
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default()
}

/// Folds one agent-call observation into a node's calls: a spawn opens a call, an exit closes the open call of its pid.
pub(super) fn observe_call(calls: &mut Vec<AgentCall>, kind: &str, fields: &serde_json::Value) {
    match kind {
        "coder.spawned" => calls.push(AgentCall {
            agent: str_field(fields, "agent"),
            model: str_field(fields, "model"),
            pid: int_field(fields, "pid"),
            image_digest: str_field(fields, "image_digest"),
            exit: None,
        }),
        "coder.exited" => {
            let pid = int_field(fields, "pid");
            if let Some(call) = calls
                .iter_mut()
                .rev()
                .find(|c| c.pid == pid && c.exit.is_none())
            {
                call.exit = Some(CallExit {
                    rc: int_field(fields, "rc"),
                    dur_ms: int_field(fields, "dur_ms"),
                    passed: int_field(fields, "criteria_passed"),
                    run: int_field(fields, "criteria_run"),
                    first_failure: str_field(fields, "first_failure"),
                    stderr_head: str_field(fields, "agent_stderr_head"),
                });
            }
        }
        _ => {}
    }
}

/// The trace lines of a journal, in journal order: one per node that finished or failed. A node the journal shows
/// making an agent observation is `coder`; any other node is `-`.
pub(super) fn node_lines(journal: &[(u64, RunEvent)]) -> Vec<NodeLine> {
    let mut started: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut observed: std::collections::BTreeMap<String, Vec<AgentCall>> =
        std::collections::BTreeMap::new();
    let mut woven: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut refusals: std::collections::BTreeMap<String, Refusal> =
        std::collections::BTreeMap::new();
    let mut lines = Vec::new();
    for (seq, event) in journal {
        match event {
            RunEvent::NodeStarted { node, at_ms } => {
                started.insert(node.to_string(), i64::from(*at_ms));
            }
            RunEvent::NodeObserved { node, kind, fields } if kind == "woven" => {
                woven.insert(node.to_string(), str_field(fields, "woven-from"));
            }
            RunEvent::NodeObserved { node, kind, fields } if kind == "worker.start.refused" => {
                observed.entry(node.to_string()).or_default();
                refusals.insert(
                    node.to_string(),
                    Refusal {
                        reason: str_field(fields, "reason"),
                        image_digest: str_field(fields, "image_digest"),
                    },
                );
            }
            RunEvent::NodeObserved { node, kind, fields } => {
                observe_call(observed.entry(node.to_string()).or_default(), kind, fields);
            }
            RunEvent::NodeFinished {
                node,
                at_ms,
                dur_ms,
                ..
            } => {
                lines.push(line(
                    *seq,
                    node.to_string(),
                    i64::from(*at_ms),
                    Some(i64::from(*dur_ms)),
                    &observed,
                    &refusals,
                    &woven,
                ));
            }
            RunEvent::NodeFailed { node, error, .. } => {
                let start = started.get(&node.to_string()).copied().unwrap_or_default();
                let mut failed = line(
                    *seq,
                    node.to_string(),
                    start,
                    None,
                    &observed,
                    &refusals,
                    &woven,
                );
                failed.failure = Some(error.clone());
                lines.push(failed);
            }
            _ => {}
        }
    }
    lines
}

pub(super) fn line(
    seq: u64,
    node: String,
    start_ms: i64,
    dur_ms: Option<i64>,
    observed: &std::collections::BTreeMap<String, Vec<AgentCall>>,
    refusals: &std::collections::BTreeMap<String, Refusal>,
    woven: &std::collections::BTreeMap<String, String>,
) -> NodeLine {
    let calls = observed.get(&node).cloned();
    NodeLine {
        seq,
        kind: if calls.is_some() { "coder" } else { "-" },
        calls: calls.unwrap_or_default(),
        refusal: refusals.get(&node).cloned(),
        woven_from: woven.get(&node).cloned(),
        failure: None,
        node,
        start_ms,
        dur_ms,
    }
}

/// The first eight hex digits of an image digest, or `-` when the run recorded none. The two words a refusal journals in
/// place of a digest pass through whole (spec 231 R2): `missing` (the runtime said the image is absent) and
/// `probe-failed` (the runtime could not answer).
pub(super) fn digest8(digest: &str) -> &str {
    if matches!(digest, "missing" | "probe-failed") {
        return digest;
    }
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    if hex.is_empty() {
        return "-";
    }
    hex.get(..8).unwrap_or(hex)
}

/// The node's failure or refusal string, uncut (spec 223 R2, 224 R2): the first criterion that failed, else the refusal's
/// reason, else the node's error. `None` when the node says none.
pub(super) fn fail_text(line: &NodeLine) -> Option<&str> {
    let exit = line.calls.last().and_then(|c| c.exit.as_ref());
    exit.map(|e| e.first_failure.as_str())
        .filter(|f| !f.is_empty())
        .or_else(|| {
            line.refusal
                .as_ref()
                .map(|r| r.reason.as_str())
                .filter(|r| !r.is_empty())
        })
        .or_else(|| line.failure.as_deref().filter(|f| !f.is_empty()))
}

/// The failure or refusal of the node's latest trace line, in full (spec 231 R1). `no-failure` when it has none.
pub(super) fn full_reason(journal: &[(u64, RunEvent)], node: &str) -> String {
    if let Some(text) = node_lines(journal)
        .iter()
        .rev()
        .find(|line| line.node == node)
        .and_then(fail_text)
    {
        return text.to_owned();
    }
    // A node that took a non-fail port with no failure fields still carries its verdict in the
    // finished event's value (measured 2026-10-20: run r258's coder node exited rc 1 and the run
    // get line said `exited=1`, but `--reason` had nothing to quote). The value IS journalled, so
    // quoting it stays a display of the journal, not of the node's self-report. A `verdict` key
    // prints alone; any other value prints whole.
    let last = journal.iter().rev().find_map(|(_, ev)| match ev {
        RunEvent::NodeFinished { node: n, value, .. } if n.as_str() == node => Some(value.clone()),
        _ => None,
    });
    let text = last.as_ref().and_then(serde_json::Value::as_str);
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        return (*t).to_owned();
    }
    last.map_or_else(
        || "no-failure".to_owned(),
        |v| serde_json::to_string(&v).unwrap_or_else(|_| "no-failure".to_owned()),
    )
}

/// One trace line (spec 217 R6). The duration falls back to the call's own when the node failed without one.
pub(super) fn trace_line(line: &NodeLine) -> String {
    let last = line.calls.last();
    let exit = last.and_then(|c| c.exit.as_ref());
    let dur = line
        .dur_ms
        .or_else(|| exit.map(|e| e.dur_ms))
        .map_or_else(|| "-".to_owned(), |d| d.to_string());
    let pid = last.map_or_else(|| "-".to_owned(), |c| c.pid.to_string());
    let refusal = line.refusal.as_ref();
    // Spec 223 R3: a node that declared an image never prints `-` here; a refusal prints its digest slot.
    let image = last
        .map(|c| digest8(&c.image_digest))
        .or_else(|| refusal.map(|r| digest8(&r.image_digest)))
        .unwrap_or("-");
    let crit = exit.map_or_else(|| "-".to_owned(), |e| format!("{}/{}", e.passed, e.run));
    // Spec 224 R2: a failed node's error is cut to 80 characters on this line; `pii run trace --reason` prints it whole.
    let fail = fail_text(line).map_or_else(|| "-".to_owned(), |f| f.chars().take(80).collect());
    let rc = exit.map_or_else(|| "-".to_owned(), |e| e.rc.to_string());
    let woven = line
        .woven_from
        .as_deref()
        .map_or_else(String::new, |origin| format!(" woven-from: {origin}"));
    format!(
        "{} {} {} {}+{} pid={pid} image={image} crit={crit} fail={fail} rc={rc}{woven}",
        line.seq, line.node, line.kind, line.start_ms, dur
    )
}

/// Spec 221 R4: one `<node> spawned=<pid|none> exited=<rc|none> refused=<reason|none>` line per node the journal
/// shows making an agent call or refusal, in the order the nodes first did so.
pub(super) fn call_lines(journal: &[(u64, RunEvent)]) -> Vec<String> {
    let mut nodes: Vec<&NodeId> = Vec::new();
    for (_, event) in journal {
        if let RunEvent::NodeObserved { node, kind, .. } = event
            && matches!(
                kind.as_str(),
                "coder.spawned" | "coder.exited" | "worker.start.refused"
            )
            && !nodes.contains(&node)
        {
            nodes.push(node);
        }
    }
    nodes
        .into_iter()
        .map(|node| {
            let calls = pii_graph::agent_calls::call_summary(journal, node);
            let pid = calls
                .spawned
                .map_or_else(|| "none".to_owned(), |p| p.to_string());
            let rc = calls
                .exit_rc
                .map_or_else(|| "none".to_owned(), |c| c.to_string());
            let refused = calls.refused.unwrap_or_else(|| "none".to_owned());
            format!("{node} spawned={pid} exited={rc} refused={refused}")
        })
        .collect()
}

/// Spec 221 R4: the coder lines of a run the daemon holds. `None` when no daemon is reachable.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn coder_lines_via_daemon(run: &str) -> Option<Result<Vec<String>, ExitCode>> {
    read_via_daemon(run).map(|loaded| loaded.map(|l| call_lines(&l.journal)))
}

/// Spec 221 R4: the coder lines of a run read from the store at `db`, the offline path.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn coder_lines_via_store(run: &str, db: &Path) -> Result<Vec<String>, ExitCode> {
    read_via_store(run, db).map(|l| call_lines(&l.journal))
}

/// The explain paragraph's sentence for one agent call (spec 217 R7).
pub(super) fn explain_call(node: &str, call: &AgentCall) -> String {
    let image = match digest8(&call.image_digest) {
        "-" => "with no image".to_owned(),
        d => format!("in image {d}"),
    };
    let Some(exit) = &call.exit else {
        return format!(
            "{node} spawned {} on model {} {image}, and no exit was recorded.",
            call.agent, call.model
        );
    };
    let failing = if exit.first_failure.is_empty() {
        String::new()
    } else {
        format!(", first failing criterion {}", exit.first_failure)
    };
    format!(
        "{node} ran {} on model {} {image} for {}ms and exited {}{}; criteria passed {}/{}{failing}.",
        call.agent,
        call.model,
        exit.dur_ms,
        exit.rc,
        agent_failure(exit),
        exit.passed,
        exit.run
    )
}

/// ` (agent failed: <stderr>)` for a non-zero exit, empty for a clean one.
///
/// The criteria ran against the tree the failed agent left, so a passed criterion beside a failed agent is not a
/// verdict; saying the agent failed, and why, keeps the two apart. The stderr is its first line only, bounded.
fn agent_failure(exit: &CallExit) -> String {
    if exit.rc == 0 {
        return String::new();
    }
    let reason = exit.stderr_head.lines().next().unwrap_or_default().trim();
    if reason.is_empty() {
        " (agent failed, no stderr)".to_owned()
    } else {
        let short: String = reason.chars().take(160).collect();
        format!(" (agent failed: {short})")
    }
}
