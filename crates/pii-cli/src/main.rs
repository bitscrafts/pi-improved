//! `pii` command-line entry point (spec F5).

#![forbid(unsafe_code)]
// reason: the CLI must write to stdout/stderr directly (spec F5.5).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

const USAGE: &str = "usage: pii [--version] | pii graph {lint|run|resume|trace} ...";

/// Renders the version banner; testable without spawning a process.
#[must_use]
pub fn version_line() -> String {
    format!("pii {}", env!("CARGO_PKG_VERSION"))
}

/// `pii graph lint <file.toml>` — validation errors/warnings to stderr, each
/// beginning with its `V<n>` code (spec F5.2/F5.3: exit 3 on lint failure).
fn cmd_graph_lint(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let spec = match pii_graph::load_toml(&src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let reg = pii_graph::NodeRegistry::builtin();
    match pii_graph::validate(&spec, &reg) {
        Ok(warnings) => {
            for w in &warnings {
                eprintln!("{w}");
            }
            if warnings.is_empty() {
                println!("{path}: OK");
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for e in &errors {
                eprintln!("{e}");
            }
            ExitCode::from(3)
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let _argv0 = args.next();
    let Some(cmd) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    // Branch on argv[1] before touching anything heavy (pass-1 §4.2).
    if cmd == "--version" || cmd == "-V" {
        println!("{}", version_line());
        return ExitCode::SUCCESS;
    }
    if cmd == "graph" {
        return cmd_graph(&mut args);
    }
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn cmd_graph(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(sub) = args.next() else {
        eprintln!("usage: pii graph {{lint|run|resume|trace}} ...");
        return ExitCode::from(2);
    };
    let sub = sub.to_str().map_or_else(|| "?".to_owned(), str::to_owned);
    if let Some(file_arg) = args.next()
        && sub == "lint"
    {
        let file = file_arg.into_string().unwrap_or_default();
        return cmd_graph_lint(&file);
    }
    if sub == "run" || sub == "resume" || sub == "trace" {
        // Journal-backed run/resume/trace needs the Store integration, which
        // lands with the scheduler run-entry pass. Cleanly report the gap.
        eprintln!(
            "error: 'graph {sub}' is not yet wired (store-backed run lands with the run engine pass)"
        );
        return ExitCode::from(2);
    }
    eprintln!("usage: pii graph {{lint|run|resume|trace}} ...");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::path::{Path, PathBuf};

    use super::version_line;

    #[test]
    fn version_line_matches_cargo_pkg_version() {
        assert_eq!(version_line(), format!("pii {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn lint_accepts_valid_diamond() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let file = root.join("examples/diamond.toml");
        write_and_lint(&file);
    }

    fn write_and_lint(p: &PathBuf) {
        // Reuse cmd_graph_lint via the real binary path is hard; instead verify
        // the loader+validator accept the example directly.
        let src = std::fs::read_to_string(p).unwrap();
        let spec = pii_graph::load_toml(&src).unwrap();
        let reg = pii_graph::NodeRegistry::builtin();
        assert!(pii_graph::validate(&spec, &reg).is_ok());
    }
}
