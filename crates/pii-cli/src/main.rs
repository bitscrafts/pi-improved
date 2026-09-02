//! `pii` command-line entry point. Phase 0: `--version` and usage only.

#![forbid(unsafe_code)]
// reason: the CLI must write to the terminal; the output abstractions
// (Ctx::out/ctx::err) that replace direct prints arrive in Phase 1.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

const USAGE: &str = "usage: pii [--version]";

/// Renders the version banner; testable without spawning a process.
#[must_use]
pub fn version_line() -> String {
    format!("pii {}", env!("CARGO_PKG_VERSION"))
}

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let _argv0 = args.next();
    match args.next() {
        Some(arg) if arg == "--version" || arg == "-V" => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::version_line;

    #[test]
    fn version_line_matches_cargo_pkg_version() {
        assert_eq!(version_line(), format!("pii {}", env!("CARGO_PKG_VERSION")));
    }
}
