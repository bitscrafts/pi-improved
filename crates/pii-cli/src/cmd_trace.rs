//! `pii graph trace <run_id> --db <path> [QUERY]` (spec F5.2).
//!
//! `QUERY` is exactly one of: (none) → ndjson dump of every journal event as
//! `{"seq":<u64>,"event":<RunEvent>}`; `--count-finished <node>`; `--node-status
//! <node>`; `--max-batch-width`; `--terminal`; `--output`; `--overlapped <a>,<b>`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus, Terminal};

use crate::cli_util::{next_value, open_store, usage_error};

const TRACE_USAGE: &str = "usage: pii graph trace <run_id> --db <path> [QUERY]\nQUERY is \
one of: (none) | --count-finished <node> | --node-status <node> | --max-batch-width | \
--terminal | --output | --overlapped <a>,<b>";

/// The single query the trace command answers.
enum Query {
    Dump,
    CountFinished(String),
    NodeStatus(String),
    MaxBatchWidth,
    Terminal,
    Output,
    Overlapped(String, String),
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
            eprintln!("error: run '{run}' not found");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let journal = match store.read_journal(&run_id) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: {e}");
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
