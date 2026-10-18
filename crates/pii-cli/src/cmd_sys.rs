//! `pii sys status|reload` and `pii run ls|start|get` (F7.1 helpers): thin
//! protocol calls, reused by the phase-3 scripts (crash/shutdown/cron).

use crate::cli_util::fail;
use std::io::Write as _;
use std::process::ExitCode;

use pii_proto::method::Method;

use crate::cmd_ctl::{next_str, print_result, run_call};

/// `pii sys status|reload`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_sys(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("status") => print_result(Method::SysStatus, serde_json::json!({})),
        Some("reload") => print_result(Method::SysReload, serde_json::json!({})),
        _ => {
            eprintln!("usage: pii sys {{status|reload}}");
            ExitCode::from(2)
        }
    }
}

/// `pii run ls [--status S] [--graph G] | start <graph> [--input J] | get <run_id>`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_run_ctl(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("ls") => run_ls(args),
        Some("start") => run_start(args),
        Some("get") => run_get(args),
        Some("resume") => run_replay(args, "resume"),
        Some("retry") => run_replay(args, "retry"),
        Some("abandon") => run_replay(args, "abandon"),
        _ => {
            eprintln!(
                "usage: pii run {{ls [--status S] [--graph G]|start <graph> [--input J]|get <run_id>\
                 |resume <run_id>|retry <run_id> --node <id>|abandon <run_id> --reason <text>}}"
            );
            ExitCode::from(2)
        }
    }
}

/// Parses `run ls` flags: `[--status S] [--graph G]`. Pure; the testable seam
/// for [`run_ls`].
fn parse_ls_flags(
    args: &mut dyn Iterator<Item = std::ffi::OsString>,
) -> (Option<String>, Option<String>) {
    let mut status: Option<String> = None;
    let mut graph: Option<String> = None;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--status" => status = next_str(args),
            "--graph" => graph = next_str(args),
            _ => {}
        }
    }
    (status, graph)
}

/// The statuses the wire can express; anything else is refused at the edge.
const STATUSES: [&str; 4] = ["running", "suspended", "succeeded", "failed"];

fn run_ls(args: &mut std::env::ArgsOs) -> ExitCode {
    let (status, graph) = parse_ls_flags(args);
    if let Some(s) = &status
        && !STATUSES.contains(&s.as_str())
    {
        fail(
            "usage",
            format!(
                "run ls: unknown --status '{s}' (accepted: {})",
                STATUSES.join("|")
            ),
        );
        return ExitCode::from(2);
    }
    let mut params = serde_json::json!({});
    if let Some(s) = &status
        && let Some(obj) = params.as_object_mut()
    {
        obj.insert(String::from("status"), serde_json::json!(s));
    }
    match run_call(Method::RunList, params) {
        Ok(v) => {
            let filtered: Vec<serde_json::Value> = v
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|h| {
                    graph.as_deref().is_none_or(|g| {
                        h.get("graph").and_then(serde_json::Value::as_str) == Some(g)
                    })
                })
                .collect();
            println!("{}", serde_json::json!(filtered));
            ExitCode::SUCCESS
        }
        Err(e) => {
            fail("sys", &e);
            ExitCode::from(2)
        }
    }
}

/// What `pii run get` says about a `run.get` answer (spec 115 R1): a run the
/// daemon holds but has not journalled yet is "accepted", a daemon that has
/// never heard of the id is "unknown" — the two must not read alike.
fn run_get_line(id: &str, answer: &Result<serde_json::Value, String>) -> String {
    match answer {
        Ok(v) if v.get("status").and_then(serde_json::Value::as_str) == Some("accepted") => {
            format!(
                "run '{id}' is accepted and in flight; its journal has not materialised yet, poll again"
            )
        }
        Ok(v) => v.to_string(),
        Err(e) if e.contains("not found") => {
            format!("run '{id}' is unknown to this daemon (not in flight, not stored)")
        }
        Err(e) => e.clone(),
    }
}

fn run_get(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(id) = next_str(args) else {
        let _ = writeln!(std::io::stderr(), "usage: pii run get <run_id>");
        return ExitCode::from(2);
    };
    let answer = run_call(Method::RunGet, serde_json::json!({"run_id": id}));
    let _ = writeln!(std::io::stdout(), "{}", run_get_line(&id, &answer));
    if answer.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

/// `pii run resume <id> | retry <id> --node N | abandon <id> --reason T`
/// (spec 186): replay is an operator action with an id, never automatic.
fn run_replay(args: &mut std::env::ArgsOs, verb: &str) -> ExitCode {
    let Some(id) = next_str(args) else {
        eprintln!("usage: pii run {verb} <run_id> ...");
        return ExitCode::from(2);
    };
    let (mut node, mut reason) = (None, None);
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--node" => node = next_str(args),
            "--reason" => reason = next_str(args),
            _ => {}
        }
    }
    let (method, params) = match verb {
        "resume" => (Method::RunResume, serde_json::json!({"run_id": id})),
        "retry" => {
            let Some(n) = node else {
                eprintln!("usage: pii run retry <run_id> --node <node_id>");
                return ExitCode::from(2);
            };
            (
                Method::RunRetry,
                serde_json::json!({"run_id": id, "node": n}),
            )
        }
        _ => {
            let Some(r) = reason else {
                eprintln!("usage: pii run abandon <run_id> --reason <text>");
                return ExitCode::from(2);
            };
            (
                Method::RunAbandon,
                serde_json::json!({"run_id": id, "reason": r}),
            )
        }
    };
    match run_call(method, params) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            fail("sys", &e);
            ExitCode::from(2)
        }
    }
}

fn run_start(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(graph) = next_str(args) else {
        eprintln!("usage: pii run start <graph> [--input J]");
        return ExitCode::from(2);
    };
    let mut input = serde_json::json!({});
    while let Some(a) = next_str(args) {
        if a == "--input"
            && let Some(raw) = next_str(args)
        {
            input = serde_json::from_str(&raw).unwrap_or(input);
        }
    }
    print_result(
        Method::RunStart,
        serde_json::json!({"graph": graph, "input": input}),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::ffi::OsString;

    use super::{parse_ls_flags, run_get_line};

    /// Spec 115 R1: in-flight and unknown never read alike.
    #[test]
    fn run_get_wording_separates_accepted_from_unknown() {
        let accepted = run_get_line("r1", &Ok(serde_json::json!({"status": "accepted"})));
        assert!(accepted.contains("accepted and in flight"), "{accepted}");
        let unknown = run_get_line("r1", &Err("not found: run 'r1'".to_owned()));
        assert!(unknown.contains("unknown to this daemon"), "{unknown}");
        assert_ne!(accepted, unknown);
        let done = run_get_line("r1", &Ok(serde_json::json!({"status": "succeeded"})));
        assert!(done.contains("succeeded"));
    }

    fn os_args(argv: &[&str]) -> std::vec::IntoIter<OsString> {
        argv.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn run_ls_parses_graph_filter() {
        let mut args = os_args(&["--graph", "foo"]);
        let (status, graph) = parse_ls_flags(&mut args);
        assert_eq!(status, None);
        assert_eq!(graph.as_deref(), Some("foo"));
        // Both filters together, plus an ignored unknown flag.
        let mut args = os_args(&["--status", "failed", "--graph", "g1", "--extra"]);
        let (status, graph) = parse_ls_flags(&mut args);
        assert_eq!(status.as_deref(), Some("failed"));
        assert_eq!(graph.as_deref(), Some("g1"));
        // No flags at all.
        let mut args = os_args(&[]);
        let (status, graph) = parse_ls_flags(&mut args);
        assert_eq!(status, None);
        assert_eq!(graph, None);
    }
}
