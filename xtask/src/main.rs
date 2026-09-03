//! `cargo xtask` dev-tools runner: deps-check, lint-files, ci.

#![forbid(unsafe_code)]
// reason: the gate runner reports violations on stderr; the output
// abstractions replacing direct prints arrive in Phase 1.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod bench;
mod deps;
mod lint_files;
mod proto_gen;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <deps-check|lint-files|proto-gen|proto-check|bench-gate [--budget-ms N]|ci>";

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

/// Runs a subcommand and reports its exit status (F8: `xtask ci` shells out
/// to fmt/clippy/test). Dev-tooling, so the Command ban does not apply.
#[allow(clippy::disallowed_methods)]
fn run_cmd(prog: &str, args: &[&str]) -> ExitCode {
    match std::process::Command::new(prog).args(args).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: failed to run {prog}: {e}");
            ExitCode::from(2)
        }
    }
}

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1);
    let rest: Vec<String> = std::env::args().skip(2).collect();
    match cmd.as_deref() {
        Some("deps-check") => report(deps::run()),
        Some("lint-files") => report(lint_files::run()),
        Some("proto-gen") => report(proto_gen::generate().map_err(|e| vec![e])),
        Some("proto-check") => report(proto_gen::check()),
        Some("bench-gate") => match bench::run_cli(&rest) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("ci") => {
            let deps_ok = report(deps::run());
            let lint_ok = report(lint_files::run());
            let proto_ok = report(proto_gen::check());
            let bench_ok = report(bench::run_cli(&rest).map_err(|e| vec![e]));
            // F8 (opus review): `cargo xtask ci` must run the full gate set —
            // fmt, clippy, tests — not just deps/lint-files/bench. This is
            // the local runner that §1.6 calls "the real gate".
            let fmt_ok = run_cmd("cargo", &["fmt", "--all", "--check"]);
            let clippy_ok = run_cmd(
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            );
            let test_ok = run_cmd("cargo", &["test", "--workspace"]);
            if deps_ok == ExitCode::SUCCESS
                && lint_ok == ExitCode::SUCCESS
                && proto_ok == ExitCode::SUCCESS
                && bench_ok == ExitCode::SUCCESS
                && fmt_ok == ExitCode::SUCCESS
                && clippy_ok == ExitCode::SUCCESS
                && test_ok == ExitCode::SUCCESS
            {
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
