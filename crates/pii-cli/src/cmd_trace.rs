//! `pii graph trace <run_id> --db <path> [QUERY]` (spec F5.2).
//!
//! `QUERY` is exactly one of: (none) → ndjson dump of every journal event as
//! `{"seq":<u64>,"event":<RunEvent>}`; `--count-finished <node>`; `--node-status
//! <node>`; `--max-batch-width`; `--terminal`; `--output`; `--overlapped <a>,<b>`;
//! `--reason <node>`.

use crate::cli_util::fail;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_proto::method::Method;
use pii_proto::run::{TraceResult, WireRunStatus};

use crate::cli_util::{next_value, open_store, usage_error};
use crate::cmd_ctl::{CallError, run_call, run_call_typed};

const TRACE_USAGE: &str = "usage: pii graph trace <run_id> --db <path> [QUERY]\nQUERY is \
one of: (none) | --count-finished <node> | --node-status <node> | --max-batch-width | \
--terminal | --output | --overlapped <a>,<b> | --reason <node>";

/// The single query the trace command answers.
enum Query {
    Dump,
    CountFinished(String),
    NodeStatus(String),
    MaxBatchWidth,
    Terminal,
    Output,
    Overlapped(String, String),
    Reason(String),
}

/// `pii graph trace ...` — answers one query over the stored journal.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_trace(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(run) = args.next().and_then(|a| a.into_string().ok()) else {
        return usage_error("missing <run_id>", TRACE_USAGE);
    };
    let db = match parse_db(args) {
        Ok(db) => db,
        Err(code) => return code,
    };
    let query = match parse_query(args) {
        Ok(query) => query,
        Err(code) => return code,
    };
    answer(&run, &db, &query)
}

/// Parses the mandatory `--db <path>` (F5.2 grammar: fixed position).
fn parse_db(args: &mut std::env::ArgsOs) -> Result<PathBuf, ExitCode> {
    let flag = next_value_or_none(args);
    match flag.as_deref() {
        Some("--db") => Ok(PathBuf::from(next_value(args, "--db", TRACE_USAGE)?)),
        _ => Err(usage_error("missing --db <path>", TRACE_USAGE)),
    }
}

fn next_value_or_none(args: &mut std::env::ArgsOs) -> Option<String> {
    args.next().and_then(|a| a.into_string().ok())
}

/// Parses the optional trailing query (exactly one, or none for a dump).
fn parse_query(args: &mut std::env::ArgsOs) -> Result<Query, ExitCode> {
    let Some(arg) = next_value_or_none(args) else {
        return Ok(Query::Dump);
    };
    match arg.as_str() {
        "--count-finished" => Ok(Query::CountFinished(next_value(
            args,
            "--count-finished",
            TRACE_USAGE,
        )?)),
        "--node-status" => Ok(Query::NodeStatus(next_value(
            args,
            "--node-status",
            TRACE_USAGE,
        )?)),
        "--max-batch-width" => Ok(Query::MaxBatchWidth),
        "--terminal" => Ok(Query::Terminal),
        "--output" => Ok(Query::Output),
        "--overlapped" => {
            let pair = next_value(args, "--overlapped", TRACE_USAGE)?;
            match pair.split_once(',') {
                Some((a, b)) => Ok(Query::Overlapped(a.to_owned(), b.to_owned())),
                None => Err(usage_error("--overlapped wants <a>,<b>", TRACE_USAGE)),
            }
        }
        "--reason" => Ok(Query::Reason(next_value(args, "--reason", TRACE_USAGE)?)),
        other => Err(usage_error(
            &format!("unknown trace query '{other}'"),
            TRACE_USAGE,
        )),
    }
}

/// Loads the run, answers the query, and returns the process exit code.
fn answer(run: &str, db: &Path, query: &Query) -> ExitCode {
    let Some(store) = open_store(db) else {
        return ExitCode::from(2);
    };
    let run_id = RunId::from(run);
    let header = match store.get_run(&run_id) {
        Ok(Some(h)) => h,
        Ok(None) => {
            fail("trace", format!("run '{run}' not found"));
            return ExitCode::from(2);
        }
        Err(e) => {
            fail("trace", &e);
            return ExitCode::from(2);
        }
    };
    let journal = match store.read_journal(&run_id) {
        Ok(j) => j,
        Err(e) => {
            fail("trace", &e);
            return ExitCode::from(2);
        }
    };
    match query {
        Query::Dump => dump(&journal),
        Query::CountFinished(node) => println!("{}", count_finished(&journal, node)),
        Query::NodeStatus(node) => println!("{}", node_status(&journal, node)),
        Query::MaxBatchWidth => println!("{}", max_batch_width(&journal)),
        Query::Terminal => println!("{}", terminal_word(&header)),
        Query::Output => print_output(&journal),
        Query::Overlapped(a, b) => println!("{}", overlapped(&journal, a, b)),
        Query::Reason(node) => println!("{}", reason_for(&journal, node)),
    }
    ExitCode::SUCCESS
}

/// Dumps every journal event as one `{"seq":..,"event":..}` ndjson line.
fn dump(journal: &[(u64, RunEvent)]) {
    for (seq, ev) in journal {
        match serde_json::to_string(ev) {
            Ok(body) => println!(r#"{{"seq":{seq},"event":{body}}}"#),
            Err(e) => println!(r#"{{"seq":{seq},"event":{{"serialize_error":"{e}"}}}}"#),
        }
    }
}

/// Counts `NodeFinished` events for `node`.
fn count_finished(journal: &[(u64, RunEvent)], node: &str) -> usize {
    journal
        .iter()
        .filter(
            |(_, ev)| matches!(ev, RunEvent::NodeFinished { node: n, .. } if n.as_str() == node),
        )
        .count()
}

/// Last terminal event wins per node; no event → `pending` (F5.2).
fn node_status(journal: &[(u64, RunEvent)], node: &str) -> &'static str {
    let mut status = "pending";
    for (_, ev) in journal {
        let next = match ev {
            RunEvent::NodeFinished { node: n, .. } if n.as_str() == node => "finished",
            RunEvent::NodeFailed { node: n, .. } if n.as_str() == node => "failed",
            RunEvent::NodeCancelled { node: n } if n.as_str() == node => "cancelled",
            RunEvent::NodeDead { node: n } if n.as_str() == node => "dead",
            _ => continue,
        };
        status = next;
    }
    status
}

/// The widest drained batch (0 when the journal has no tick).
fn max_batch_width(journal: &[(u64, RunEvent)]) -> u32 {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::TickStarted { width, .. } => Some(*width),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Maps the run header status to its F5.2 word.
fn terminal_word(header: &RunHeader) -> &'static str {
    match header.status {
        RunStatus::Running => "running",
        RunStatus::Suspended => "suspended",
        RunStatus::Succeeded => "success",
        RunStatus::Failed => "failure",
        RunStatus::Refused => "refused",
    }
}

/// Prints the terminal output JSON (Success → output; Failure → error;
/// Suspended → key; no terminal yet → `null`).
fn print_output(journal: &[(u64, RunEvent)]) {
    let terminal = journal.iter().rev().find_map(|(_, ev)| match ev {
        RunEvent::Finished { terminal, .. } => Some(terminal),
        _ => None,
    });
    match terminal {
        Some(Terminal::Success { output }) => println!("{output}"),
        Some(Terminal::Failure { error }) => println!("{}", serde_json::json!(error)),
        Some(Terminal::Suspended { key }) => println!("{}", serde_json::json!(key)),
        None => println!("null"),
    }
}

/// `a.at_ms < b.at_ms + b.dur_ms && b.at_ms < a.at_ms + a.dur_ms` over the
/// two `NodeFinished` events (F5.2); `no` when either never finished.
fn overlapped(journal: &[(u64, RunEvent)], a: &str, b: &str) -> &'static str {
    let Some((a_at, a_dur)) = last_window(journal, a) else {
        return "no";
    };
    let Some((b_at, b_dur)) = last_window(journal, b) else {
        return "no";
    };
    let a_start = u64::from(a_at);
    let a_end = a_start + u64::from(a_dur);
    let b_start = u64::from(b_at);
    let b_end = b_start + u64::from(b_dur);
    if a_start < b_end && b_start < a_end {
        "yes"
    } else {
        "no"
    }
}

/// `(at_ms, dur_ms)` of the latest `NodeFinished` for `node`.
fn last_window(journal: &[(u64, RunEvent)], node: &str) -> Option<(u32, u32)> {
    journal.iter().rev().find_map(|(_, ev)| match ev {
        RunEvent::NodeFinished {
            node: n,
            at_ms,
            dur_ms,
            ..
        } if n.as_str() == node => Some((*at_ms, *dur_ms)),
        _ => None,
    })
}

/// The failover reason on the fail port (A5): the latest `NodeFinished` for
/// `node` decides — same "last event wins" convention `node_status` and
/// `last_window` use, so a node that failed once and succeeded on retry
/// reports `no-failure`, not the stale attempt's reason.
///
/// No `NodeFinished` at all, or the latest one is not on the `fail` port →
/// `no-failure`. On `fail`, the stored `value` (`FailurePayload`) is printed
/// as-is — byte-identical to what the journal holds, not reformatted.
fn reason_for(journal: &[(u64, RunEvent)], node: &str) -> String {
    let latest = journal.iter().rev().find_map(|(_, ev)| match ev {
        RunEvent::NodeFinished {
            node: n,
            port,
            value,
            ..
        } if n.as_str() == node => Some((port.as_str().to_owned(), value.clone())),
        _ => None,
    });
    match latest {
        Some((port, value)) if port == "fail" => {
            serde_json::to_string(&value).unwrap_or_else(|_| "no-failure".to_owned())
        }
        _ => "no-failure".to_owned(),
    }
}

// --- Spec 217: `pii run trace` and `pii run explain`. Built from the run's journal events only: a node's returned
// value is never read here, so a node cannot report its own verdict into the trace.

/// One agent call a node made, as the journal recorded it: its spawn, and its exit once one landed.
#[derive(Debug, Clone)]
struct AgentCall {
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
}

/// One trace line: a node that finished or failed, with the agent calls it made up to that point.
#[derive(Debug, Clone)]
struct NodeLine {
    seq: u64,
    node: String,
    kind: &'static str,
    start_ms: i64,
    dur_ms: Option<i64>,
    calls: Vec<AgentCall>,
    /// The `<fragment>#<node>` origin of a woven node (spec 219 R4).
    woven_from: Option<String>,
}

fn str_field(fields: &serde_json::Value, key: &str) -> String {
    fields
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn int_field(fields: &serde_json::Value, key: &str) -> i64 {
    fields
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default()
}

/// Folds one agent-call observation into a node's calls: a spawn opens a call, an exit closes the open call of its pid.
fn observe_call(calls: &mut Vec<AgentCall>, kind: &str, fields: &serde_json::Value) {
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
                });
            }
        }
        _ => {}
    }
}

/// The trace lines of a journal, in journal order: one per node that finished or failed. A node the journal shows
/// making an agent observation is `coder`; any other node is `-`.
fn node_lines(journal: &[(u64, RunEvent)]) -> Vec<NodeLine> {
    let mut started: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut observed: std::collections::BTreeMap<String, Vec<AgentCall>> =
        std::collections::BTreeMap::new();
    let mut woven: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut lines = Vec::new();
    for (seq, event) in journal {
        match event {
            RunEvent::NodeStarted { node, at_ms } => {
                started.insert(node.to_string(), i64::from(*at_ms));
            }
            RunEvent::NodeObserved { node, kind, fields } if kind == "woven" => {
                woven.insert(node.to_string(), str_field(fields, "woven-from"));
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
                    &woven,
                ));
            }
            RunEvent::NodeFailed { node, .. } => {
                let start = started.get(&node.to_string()).copied().unwrap_or_default();
                lines.push(line(*seq, node.to_string(), start, None, &observed, &woven));
            }
            _ => {}
        }
    }
    lines
}

fn line(
    seq: u64,
    node: String,
    start_ms: i64,
    dur_ms: Option<i64>,
    observed: &std::collections::BTreeMap<String, Vec<AgentCall>>,
    woven: &std::collections::BTreeMap<String, String>,
) -> NodeLine {
    let calls = observed.get(&node).cloned();
    NodeLine {
        seq,
        kind: if calls.is_some() { "coder" } else { "-" },
        calls: calls.unwrap_or_default(),
        woven_from: woven.get(&node).cloned(),
        node,
        start_ms,
        dur_ms,
    }
}

/// The first eight hex digits of an image digest, or `-` when the run recorded none.
fn digest8(digest: &str) -> &str {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    if hex.is_empty() {
        return "-";
    }
    hex.get(..8).unwrap_or(hex)
}

/// One trace line (spec 217 R6). The duration falls back to the call's own when the node failed without one.
fn trace_line(line: &NodeLine) -> String {
    let last = line.calls.last();
    let exit = last.and_then(|c| c.exit.as_ref());
    let dur = line
        .dur_ms
        .or_else(|| exit.map(|e| e.dur_ms))
        .map_or_else(|| "-".to_owned(), |d| d.to_string());
    let pid = last.map_or_else(|| "-".to_owned(), |c| c.pid.to_string());
    let image = last.map_or("-", |c| digest8(&c.image_digest));
    let crit = exit.map_or_else(|| "-".to_owned(), |e| format!("{}/{}", e.passed, e.run));
    let fail = exit
        .map(|e| e.first_failure.as_str())
        .filter(|f| !f.is_empty())
        .unwrap_or("-");
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
fn call_lines(journal: &[(u64, RunEvent)]) -> Vec<String> {
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
fn explain_call(node: &str, call: &AgentCall) -> String {
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
        "{node} ran {} on model {} {image} for {}ms and exited {}; criteria passed {}/{}{failing}.",
        call.agent, call.model, exit.dur_ms, exit.rc, exit.passed, exit.run
    )
}

/// Where the journal a `pii run trace|explain` read came from (spec 220 R4): the daemon, or a store opened directly.
enum Source {
    Daemon,
    Store(PathBuf),
}

impl Source {
    /// The line that says which source answered, so a reader can tell where the lines came from.
    fn marker(&self) -> String {
        match self {
            Self::Daemon => "via=daemon".to_owned(),
            Self::Store(db) => format!("via=store db={}", db.display()),
        }
    }
}

/// A run as `pii run trace|explain` shows it: its header and journal, read through one [`Source`].
struct Loaded {
    graph: String,
    status: String,
    dur_ms: u32,
    terminal: bool,
    journal: Vec<(u64, RunEvent)>,
    source: Source,
}

/// Asks the daemon for the run through `run.trace` (spec 220 R1). `None` when no daemon is reachable, so the caller
/// falls back to the store; `Some(Err)` when the daemon answered and could not give the run.
fn read_via_daemon(run: &str) -> Option<Result<Loaded, ExitCode>> {
    match run_call_typed(Method::RunTrace, serde_json::json!({ "run_id": run })) {
        Ok(reply) => Some(daemon_loaded(reply)),
        Err(CallError::Down(_)) => None,
        Err(CallError::Failed(msg)) => Some(Err(daemon_refused(&msg))),
    }
}

/// Decodes a `run.trace` reply. Each journal entry is the `{"seq","event"}` object the store dump prints (spec 220 R3).
fn daemon_loaded(reply: serde_json::Value) -> Result<Loaded, ExitCode> {
    let Ok(trace) = serde_json::from_value::<TraceResult>(reply) else {
        fail("trace", "the daemon's reply is not a run trace");
        return Err(ExitCode::FAILURE);
    };
    let mut journal = Vec::with_capacity(trace.journal.len());
    for entry in &trace.journal {
        let seq = entry.get("seq").and_then(serde_json::Value::as_u64);
        let event = entry.get("event").cloned().unwrap_or_default();
        let (Some(seq), Ok(event)) = (seq, serde_json::from_value::<RunEvent>(event)) else {
            fail(
                "trace",
                "the daemon sent a journal entry that does not decode",
            );
            return Err(ExitCode::FAILURE);
        };
        journal.push((seq, event));
    }
    let status = trace.header.status;
    Ok(Loaded {
        graph: trace.header.graph,
        status: format!("{status:?}").to_lowercase(),
        dur_ms: trace.header.dur_ms,
        terminal: matches!(
            status,
            WireRunStatus::Succeeded | WireRunStatus::Failed | WireRunStatus::Refused
        ),
        journal,
        source: Source::Daemon,
    })
}

/// The daemon answered and refused the run. Its message is printed with the store it consulted and the runs it holds,
/// so "no run" can be told from "wrong store" (spec 220 R2).
fn daemon_refused(msg: &str) -> ExitCode {
    let store = run_call(Method::SysStatus, serde_json::json!({}))
        .ok()
        .and_then(|status| {
            let stores = status.get("stores")?.as_array()?;
            let main = stores
                .iter()
                .find(|s| s.get("name").and_then(serde_json::Value::as_str) == Some("main"))?;
            main.get("path")?.as_str().map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let runs = run_call(Method::RunList, serde_json::json!({}))
        .ok()
        .and_then(|runs| runs.as_array().map(Vec::len))
        .map_or_else(|| "unknown".to_owned(), |n| n.to_string());
    fail(
        "run",
        format!("{msg}; consulted daemon store {store}, which has {runs} runs"),
    );
    ExitCode::FAILURE
}

/// Reads the run straight from the store at `db`: the offline path, used only when no daemon is reachable (spec 220 R1).
fn read_via_store(run: &str, db: &Path) -> Result<Loaded, ExitCode> {
    let Some(store) = open_store(db) else {
        return Err(ExitCode::FAILURE);
    };
    let id = RunId::from(run);
    let header = match store.get_run(&id) {
        Ok(Some(header)) => header,
        Ok(None) => {
            fail(
                "run",
                format!(
                    "no run {run} in store {} (no daemon reachable; opened directly)",
                    db.display()
                ),
            );
            return Err(ExitCode::FAILURE);
        }
        Err(e) => {
            fail("store", e);
            return Err(ExitCode::FAILURE);
        }
    };
    let journal = match store.read_journal(&id) {
        Ok(journal) => journal,
        Err(e) => {
            fail("store", e);
            return Err(ExitCode::FAILURE);
        }
    };
    Ok(Loaded {
        graph: header.graph.clone(),
        status: format!("{:?}", header.status).to_lowercase(),
        dur_ms: header.dur_ms,
        terminal: matches!(
            header.status,
            RunStatus::Succeeded | RunStatus::Failed | RunStatus::Refused
        ),
        journal,
        source: Source::Store(db.to_path_buf()),
    })
}

/// Prints a loaded run for `verb` (`trace` or `explain`), ending with the line that names its source (spec 220 R4).
fn print_run(verb: &str, run: &str, loaded: &Loaded) -> ExitCode {
    let lines = node_lines(&loaded.journal);
    let mut out = std::io::stdout();
    if verb == "explain" {
        let mut sentences = vec![format!(
            "Run {run} of {} {} in {}ms.",
            loaded.graph, loaded.status, loaded.dur_ms
        )];
        for line in &lines {
            sentences.extend(line.calls.iter().map(|c| explain_call(&line.node, c)));
        }
        let _ = writeln!(out, "{}", sentences.join(" "));
        let _ = writeln!(out, "{}", loaded.source.marker());
        return ExitCode::SUCCESS;
    }
    for line in &lines {
        let _ = writeln!(out, "{}", trace_line(line));
    }
    let calls = loaded
        .journal
        .iter()
        .filter(
            |(_, e)| matches!(e, RunEvent::NodeObserved { kind, .. } if kind == "coder.spawned"),
        )
        .count();
    let orphans = if loaded.terminal {
        pii_graph::agent_calls::unmatched_spawns(&loaded.journal).len()
    } else {
        0
    };
    let _ = writeln!(
        out,
        "trace: {} nodes · {calls} agent calls · {orphans} orphans",
        lines.len()
    );
    let _ = writeln!(out, "{}", loaded.source.marker());
    ExitCode::SUCCESS
}

/// `pii run trace <run_id> | explain <run_id> [--db <path>]` (spec 217 R6–R7, spec 220). The daemon answers first;
/// the store (`--db`, else `$PII_RUNTIME_DIR/store.redb`) is read only when no daemon is reachable.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_run_observed(verb: &str, args: &mut std::env::ArgsOs) -> ExitCode {
    let usage = format!("usage: pii run {verb} <run_id> [--db <path>]");
    let Some(run) = args.next().and_then(|a| a.into_string().ok()) else {
        return usage_error("missing <run_id>", &usage);
    };
    let mut db = None;
    while let Some(flag) = next_value_or_none(args) {
        if flag != "--db" {
            return usage_error(&format!("unknown flag {flag}"), &usage);
        }
        match next_value(args, "--db", &usage) {
            Ok(path) => db = Some(PathBuf::from(path)),
            Err(code) => return code,
        }
    }
    let loaded = if let Some(outcome) = read_via_daemon(&run) {
        outcome
    } else {
        let db = db.or_else(|| {
            std::env::var_os("PII_RUNTIME_DIR").map(|d| PathBuf::from(d).join("store.redb"))
        });
        let Some(db) = db else {
            return usage_error("no store: pass --db <path> or set PII_RUNTIME_DIR", &usage);
        };
        read_via_store(&run, &db)
    };
    match loaded {
        Ok(loaded) => print_run(verb, &run, &loaded),
        Err(code) => code,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::reason_for;
    use pii_abi::{NodeId, PortName, RunEvent};

    /// A `NodeFinished` event for `node` on `port` carrying `value`.
    fn finished(node: &str, port: &str, value: serde_json::Value) -> RunEvent {
        RunEvent::NodeFinished {
            node: NodeId::from(node),
            port: PortName::from(port),
            value,
            value_hash: "h".to_string(),
            at_ms: 0,
            dur_ms: 1,
        }
    }

    fn journal(events: Vec<RunEvent>) -> Vec<(u64, RunEvent)> {
        events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64, e))
            .collect()
    }

    #[test]
    fn trace_reason_prints_no_failure_for_never_failed_node() {
        let j = journal(vec![finished("n", "ok", serde_json::json!({"x": 1}))]);
        assert_eq!(reason_for(&j, "n"), "no-failure");
    }

    #[test]
    fn trace_reason_prints_failure_payload_for_fail_port() {
        let payload = serde_json::json!({
            "status": 503,
            "reason": "overloaded",
            "provider": "llamacpp-server"
        });
        let j = journal(vec![finished("n", "fail", payload.clone())]);
        // Byte-identical to the stored value, not reformatted.
        assert_eq!(
            reason_for(&j, "n"),
            serde_json::to_string(&payload).unwrap_or_default()
        );
    }

    #[test]
    fn trace_reason_uses_latest_event_across_retries() {
        let j = journal(vec![
            finished(
                "n",
                "fail",
                serde_json::json!({"status": 503, "reason": "overloaded"}),
            ),
            finished("n", "ok", serde_json::json!({"recovered": true})),
        ]);
        assert_eq!(reason_for(&j, "n"), "no-failure");
    }

    #[test]
    fn trace_reason_missing_node_prints_no_failure() {
        let j = journal(vec![finished("n", "ok", serde_json::json!({}))]);
        assert_eq!(reason_for(&j, "missing"), "no-failure");
    }
}
