//! `pii-evolve` binary: launches one boss-watch cycle on the daemon.
//!
//! Usage: `pii-evolve <state-dir> [--socket PATH]`
//!
//! `--socket PATH` overrides the daemon's UDS control socket; when omitted,
//! `$PII_RUNTIME_DIR/control.sock` is used, falling back to
//! `~/.pii/runtime/control.sock`.

#![forbid(unsafe_code)]
// reason: the CLI binary reports outcomes on stderr.
#![allow(clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use pii_evolve::driver::{self, BossLaunch};

const USAGE: &str = "usage: pii-evolve <state-dir> [--socket PATH]";

fn default_socket() -> PathBuf {
    if let Some(dir) = std::env::var_os("PII_RUNTIME_DIR") {
        return PathBuf::from(dir).join("control.sock");
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".pii/runtime/control.sock")
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(state_dir) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };

    let mut socket = default_socket();
    while let Some(arg) = args.next() {
        if arg != "--socket" {
            eprintln!("unknown argument: {arg}\n{USAGE}");
            return ExitCode::FAILURE;
        }
        let Some(path) = args.next() else {
            eprintln!("--socket requires a PATH argument\n{USAGE}");
            return ExitCode::FAILURE;
        };
        socket = PathBuf::from(path);
    }

    let cfg = BossLaunch::new(socket, PathBuf::from(state_dir));
    match driver::run_cycle(&cfg) {
        Ok(launched) => {
            eprintln!(
                "pii-evolve: launched {} (cycle {})",
                launched.run_id, launched.cycle
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pii-evolve: {e}");
            ExitCode::FAILURE
        }
    }
}
