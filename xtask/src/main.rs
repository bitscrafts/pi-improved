//! `cargo xtask` dev-tools runner: deps-check, lint-files, ci.

#![forbid(unsafe_code)]
// reason: the gate runner reports violations on stderr; the output
// abstractions replacing direct prints arrive in Phase 1.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod bench;
mod deps;
mod lint_files;

use std::process::ExitCode;

const USAGE: &str =
    "usage: cargo xtask <deps-check|lint-files|bench-gate [--budget-ms N]|ci>";

fn report(result: Result<(), Vec<String>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(violations) => {
            for line in violations {
                eprintln!("error: {line}");
            }
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1);
    match cmd.as_deref() {
        Some("deps-check") => report(deps::run()),
        Some("lint-files") => report(lint_files::run()),
        Some("ci") => {
            let a = report(deps::run());
            let b = report(lint_files::run());
            if a == ExitCode::SUCCESS && b == ExitCode::SUCCESS {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
