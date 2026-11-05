//! `pii queue {add|ls|claim}` (spec 352 R5): the daemon's queue, from outside. Each subcommand is one RPC to the
//! daemon, which owns the store; none opens it.

use std::process::ExitCode;

use pii_proto::QueueAddParams;
use pii_proto::method::Method;

use crate::cmd_ctl::{next_str, print_result};

const USAGE: &str = "usage: pii queue add --id <id> --spec <path> --title <text> --criteria <cmd> [--criteria <cmd>]... [--touches <path>]...\n       pii queue ls\n       pii queue claim --by <name>";

/// `pii queue ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_queue(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("add") => cmd_add(args),
        Some("ls") => {
            warn!("rows: state, and for a suspended run suspended_ms against deadline_ms");
            print_result(Method::QueueList, serde_json::json!({}))
        }
        Some("claim") => cmd_claim(args),
        _ => {
            warn!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn cmd_add(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut p = QueueAddParams::default();
    while let Some(flag) = next_str(args) {
        if flag == "--help" || flag == "-h" {
            say!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        let Some(value) = next_str(args) else {
            warn!("{USAGE}");
            return ExitCode::from(2);
        };
        match flag.as_str() {
            "--id" => p.id = value,
            "--spec" => p.spec = value,
            "--title" => p.title = value,
            "--criteria" => p.criteria.push(value),
            "--touches" => p.touches.push(value),
            _ => {
                warn!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    serde_json::to_value(p).map_or_else(
        |_| ExitCode::from(2),
        |params| print_result(Method::QueueAdd, params),
    )
}

fn cmd_claim(args: &mut std::env::ArgsOs) -> ExitCode {
    let (flag, by) = (next_str(args), next_str(args));
    if let (Some("--by"), Some(by)) = (flag.as_deref(), by) {
        return print_result(Method::QueueClaim, serde_json::json!({ "by": by }));
    }
    warn!("{USAGE}");
    ExitCode::from(2)
}
