//! `pii worker-exec` (F3.8): the reference worker, speaking F3.2 frames on
//! stdio. Ops: `echo`, `sleep`, `crash`, `info`. Must not depend on the
//! engine — it is the fixture for E12/E13/E14.

#![forbid(unsafe_code)]
// reason: worker-exec answers frames directly; crash uses abort via std.
#![allow(clippy::print_stdout, clippy::print_stderr)] // reason: stdio IS the wire

use std::io::{BufRead, Write};
use std::process::ExitCode;

use pii_worker::frame::{Hello, WORKER_PROTOCOL, WorkerErr, WorkerFrame, parse_frame, write_frame};

/// `pii worker-exec --name <n> [--tag <t>]` (F3.8).
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the widest
// reachable form; pedantic disagrees with unreachable_pub's advice here.
pub(crate) fn cmd_worker_exec() -> ExitCode {
    let (name, tag) = parse_flags();
    run_worker(&name, &tag)
}

/// Parses `--name` / `--tag` from argv (defaults `w` / empty).
fn parse_flags() -> (String, String) {
    let mut name = String::from("w");
    let mut tag = String::new();
    let mut args = std::env::args().skip(2);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => name = args.next().unwrap_or(name),
            "--tag" => tag = args.next().unwrap_or_default(),
            _ => {}
        }
    }
    (name, tag)
}

/// The frame loop: read one frame, answer it (F3.2). A malformed frame is
/// skipped, never fatal (the supervisor is the one that kills bad workers).
#[allow(clippy::too_many_lines)]
// reason: one flat dispatch loop is clearer
// than scattering the six ops across functions; the ops ARE the structure.
#[allow(clippy::significant_drop_tightening)] // reason: the stdin guard must
// live for the whole serve loop — stdin IS the wire; there is no shorter
// correct scope (same shape as ProcCap::run's drained pipes).
fn run_worker(name: &str, tag: &str) -> ExitCode {
    let pid = std::process::id();
    let mut out = std::io::stdout();
    let mut line = Vec::new();
    let stdin = std::io::stdin();
    let mut locked = stdin.lock();
    loop {
        line.clear();
        match locked.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return ExitCode::SUCCESS,
            Ok(_) => {}
        }
        let Ok(frame) = parse_frame(
            std::str::from_utf8(&line)
                .unwrap_or("")
                .trim_end_matches(['\n', '\r']),
        ) else {
            continue;
        };
        let WorkerFrame::Req { id, op, args } = frame else {
            continue;
        };
        let res = match op.as_str() {
            "hello" => WorkerFrame::Res {
                id,
                ok: Some(
                    serde_json::to_value(&Hello {
                        name: name.to_owned(),
                        tag: tag.to_owned(),
                        pid,
                        protocol: String::from(WORKER_PROTOCOL),
                    })
                    .unwrap_or_default(),
                ),
                err: None,
            },
            "info" => WorkerFrame::Res {
                id,
                ok: Some(serde_json::json!({"name": name, "tag": tag, "pid": pid})),
                err: None,
            },
            "echo" => WorkerFrame::Res {
                id,
                ok: Some(args.get("v").cloned().unwrap_or(serde_json::Value::Null)),
                err: None,
            },
            "sleep" => {
                let ms = args
                    .get("ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                std::thread::sleep(std::time::Duration::from_millis(ms));
                WorkerFrame::Res {
                    id,
                    ok: Some(serde_json::json!({"slept": ms})),
                    err: None,
                }
            }
            "crash" => {
                // Immediate death WITHOUT answering (F3.8). abort is the
                // sanctioned shape: SIGABRT, no cleanup, no response frame.
                std::process::abort();
            }
            "shutdown" => {
                let f = write_frame(&WorkerFrame::Res {
                    id,
                    ok: Some(serde_json::json!({"bye": true})),
                    err: None,
                });
                if let Ok(b) = f
                    && out.write_all(&b).is_ok()
                {
                    let _ = out.flush();
                }
                return ExitCode::SUCCESS;
            }
            other => WorkerFrame::Res {
                id,
                ok: None,
                err: Some(WorkerErr {
                    code: String::from("unknown_op"),
                    message: format!("unknown op '{other}'"),
                }),
            },
        };
        if let Ok(b) = write_frame(&res)
            && out.write_all(&b).is_ok()
        {
            let _ = out.flush();
        }
    }
}
