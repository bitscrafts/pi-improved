//! `pii graph trace <run_id> --db <path> [QUERY]` (spec F5.2).
//!
//! `QUERY` is exactly one of: (none) → ndjson dump of every journal event as
//! `{"seq":<u64>,"event":<RunEvent>}`; `--count-finished <node>`; `--node-status
//! <node>`; `--max-batch-width`; `--terminal`; `--output`; `--overlapped <a>,<b>`;
//! `--reason <node>`.

use crate::cli_util::fail;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_abi::{RunEvent, RunHeader, RunId, RunStatus, Terminal};

use crate::cli_util::{next_value, open_store, usage_error};

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
            "provider": "ollama"
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
