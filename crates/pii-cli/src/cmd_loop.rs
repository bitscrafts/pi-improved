//! `pii loop emit|events` (spec 173): the autonomy loop's lifecycle lines as events, and one reader
//! that shows them beside the daemon's own. `emit` always keeps the event locally; it exits 3 when
//! the daemon did not answer, so the caller can record the gap. It never waits on the daemon beyond
//! the usual connect bound.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use pii_learn::loop_events::{LoopEvent, append, merged, render};
use pii_proto::method::Method;

use crate::cmd_ctl::{next_str, run_call};

const USAGE: &str = "usage: pii loop emit --cycle <id> --kind <kind> [--detail <text>] [--file <f>] | pii loop events [--last <n>] [--file <f>] [--log <daemon log>]";

fn default_file() -> PathBuf {
    std::env::var_os("PII_LOOP_EVENTS").map_or_else(
        || PathBuf::from("_tmp/autonomy/loop-events.ndjson"),
        PathBuf::from,
    )
}

// reason: a CLI stamp of wall-clock time; there is no replayed Ctx in this process.
#[allow(clippy::disallowed_methods)]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

/// `pii loop {emit|events}`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_loop(args: &mut std::env::ArgsOs) -> ExitCode {
    let sub = next_str(args);
    let (mut file, mut log, mut cycle, mut kind, mut detail) = (None, None, None, None, None);
    let mut last = 20_usize;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--file" => file = next_str(args).map(PathBuf::from),
            "--log" => log = next_str(args).map(PathBuf::from),
            "--cycle" => cycle = next_str(args),
            "--kind" => kind = next_str(args),
            "--detail" => detail = next_str(args),
            "--last" => last = next_str(args).and_then(|n| n.parse().ok()).unwrap_or(last),
            _ => return usage(),
        }
    }
    let file = file.unwrap_or_else(default_file);
    match (sub.as_deref(), cycle, kind) {
        (Some("emit"), Some(cycle), Some(kind)) => {
            emit(&file, cycle, kind, detail.unwrap_or_default())
        }
        (Some("events"), _, _) => {
            let mut out = std::io::stdout();
            for l in merged(&file, log.as_deref(), last) {
                let _ = writeln!(out, "{}", render(&l));
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn emit(file: &std::path::Path, cycle: String, kind: String, mut detail: String) -> ExitCode {
    let up = run_call(Method::SysStatus, serde_json::json!({})).is_ok();
    if !up {
        if !detail.is_empty() {
            detail.push(' ');
        }
        detail.push_str("daemon=down");
    }
    let ev = LoopEvent {
        ts_ms: now_ms(),
        cycle,
        kind,
        detail,
    };
    if let Err(e) = append(file, &ev) {
        let _ = writeln!(std::io::stderr(), "error: cannot record loop event: {e}");
        return ExitCode::from(2);
    }
    if up {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

fn usage() -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{USAGE}");
    ExitCode::from(2)
}
