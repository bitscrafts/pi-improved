//! `cargo xtask` dev-tools runner: deps-check, lint-files, ci.

#![forbid(unsafe_code)]
// reason: the gate runner reports violations on stderr; the output
// abstractions replacing direct prints arrive in Phase 1.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod bench;
mod deps;
mod lint_files;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <deps-check|lint-files|bench-gate [--budget-ms N]|ci>";

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
    let rest: Vec<String> = std::env::args().skip(2).collect();
    match cmd.as_deref() {
        Some("deps-check") => report(deps::run()),
        Some("lint-files") => report(lint_files::run()),
        Some("bench-gate") => match bench::run_cli(&rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("ci") => {
            let a = report(deps::run());
            let b = report(lint_files::run());
            let c = report(bench::run_cli(&rest).map_err(|e| vec![e]));
            if a == ExitCode::SUCCESS && b == ExitCode::SUCCESS && c == ExitCode::SUCCESS {
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
