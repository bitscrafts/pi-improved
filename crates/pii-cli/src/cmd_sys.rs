//! `pii sys status|reload` and `pii run ls|start` (F7.1 helpers): thin
//! protocol calls, reused by the phase-3 scripts (crash/shutdown/cron).

use crate::cli_util::fail;
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

/// `pii run ls [--status S] [--graph G] | start <graph> [--input J]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_run_ctl(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("ls") => run_ls(args),
        Some("start") => run_start(args),
        _ => {
            eprintln!("usage: pii run {{ls [--status S] [--graph G]|start <graph> [--input J]}}");
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

    use super::parse_ls_flags;

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
