//! Spec 220 run loading and printing for `pii run trace|explain`: the daemon answers first, the store only
//! when no daemon is reachable.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_abi::{RunEvent, RunId, RunStatus};
use pii_proto::method::Method;
use pii_proto::run::{TraceResult, WireRunStatus};

use super::analysis::{explain_call, full_reason, graph_call_lines, node_lines, trace_line};
use super::next_value_or_none;
use crate::cli_util::fail;
use crate::cli_util::{next_value, open_store, usage_error};
use crate::cmd_ctl::{CallError, run_call, run_call_typed};

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
pub(super) struct Loaded {
    graph: String,
    status: String,
    dur_ms: u32,
    terminal: bool,
    pub(super) journal: Vec<(u64, RunEvent)>,
    source: Source,
}

/// Asks the daemon for the run through `run.trace` (spec 220 R1). `None` when no daemon is reachable, so the caller
/// falls back to the store; `Some(Err)` when the daemon answered and could not give the run.
pub(super) fn read_via_daemon(run: &str) -> Option<Result<Loaded, ExitCode>> {
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
pub(super) fn read_via_store(run: &str, db: &Path) -> Result<Loaded, ExitCode> {
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
            // Spec 224 R3: the failing node's reason, in full.
            if let Some(error) = &line.failure {
                sentences.push(format!("{} failed: {error}.", line.node));
            }
        }
        let _ = writeln!(out, "{}", sentences.join(" "));
        let _ = writeln!(out, "{}", loaded.source.marker());
        return ExitCode::SUCCESS;
    }
    for call in graph_call_lines(&loaded.journal) {
        let _ = writeln!(out, "{call}");
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

/// `pii run trace <run_id> | explain <run_id> [--db <path>]` (spec 217 R6–R7, spec 220), and `pii run trace <run_id>
/// --reason <node>` (spec 231 R1). The daemon answers first; the store (`--db`, else `$PII_RUNTIME_DIR/store.redb`) is
/// read only when no daemon is reachable.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_run_observed(verb: &str, args: &mut std::env::ArgsOs) -> ExitCode {
    let usage = if verb == "trace" {
        "usage: pii run trace <run_id> [--db <path>] [--reason <node>]".to_owned()
    } else {
        format!("usage: pii run {verb} <run_id> [--db <path>]")
    };
    let Some(run) = args.next().and_then(|a| a.into_string().ok()) else {
        return usage_error("missing <run_id>", &usage);
    };
    let mut db = None;
    let mut reason = None;
    while let Some(flag) = next_value_or_none(args) {
        match flag.as_str() {
            "--db" => match next_value(args, "--db", &usage) {
                Ok(path) => db = Some(PathBuf::from(path)),
                Err(code) => return code,
            },
            "--reason" if verb == "trace" => match next_value(args, "--reason", &usage) {
                Ok(node) => reason = Some(node),
                Err(code) => return code,
            },
            _ => return usage_error(&format!("unknown flag {flag}"), &usage),
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
    match (loaded, reason) {
        (Ok(loaded), Some(node)) => {
            println!("{}", full_reason(&loaded.journal, &node));
            ExitCode::SUCCESS
        }
        (Ok(loaded), None) => print_run(verb, &run, &loaded),
        (Err(code), _) => code,
    }
}
