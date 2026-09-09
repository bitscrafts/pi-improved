//! `pii-evolve` binary: runs one boss cycle against a state directory.
//!
//! Usage: `pii-evolve <state-dir> [--policy FILE]`
//!
//! `--policy FILE` is a plain-text file whose (trimmed) contents name the
//! workspace root the deterministic gates run from; when omitted, the
//! process's current directory is used.

#![forbid(unsafe_code)]
// reason: the CLI binary reports outcomes on stderr.
#![allow(clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use pii_evolve::driver::{self, CycleOutcome, GatePolicy};

const USAGE: &str = "usage: pii-evolve <state-dir> [--policy FILE]";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(state_dir) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };

    let mut workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    while let Some(arg) = args.next() {
        if arg != "--policy" {
            eprintln!("unknown argument: {arg}\n{USAGE}");
            return ExitCode::FAILURE;
        }
        let Some(file) = args.next() else {
            eprintln!("--policy requires a FILE argument\n{USAGE}");
            return ExitCode::FAILURE;
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    workspace_root = PathBuf::from(trimmed);
                }
            }
            Err(e) => {
                eprintln!("failed to read policy file {file}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let policy = GatePolicy::default_gates(workspace_root);
    // A default budget: plenty (10 gate calls, 100s) so normal cycles run, but
    // never unbounded — the boss must be budget-bound (008 §6).
    let budget = pii_core::Budget::new(10_000, 10, 0, 100_000);
    match driver::run_cycle(&PathBuf::from(state_dir), &policy, budget) {
        Ok(CycleOutcome::Ok) => ExitCode::SUCCESS,
        Ok(CycleOutcome::PendingHuman(gate)) => {
            eprintln!("pii-evolve: gate '{gate}' failed; suspended for human review");
            ExitCode::FAILURE
        }
        Ok(CycleOutcome::BudgetExhausted) => {
            eprintln!("pii-evolve: budget exhausted; not spending further");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("pii-evolve: {e}");
            ExitCode::FAILURE
        }
    }
}
