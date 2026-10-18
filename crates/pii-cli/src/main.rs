//! `pii` command-line entry point (spec F5).

#![forbid(unsafe_code)]
// reason: the CLI must write to stdout/stderr directly (spec F5.5).
// spec 67 qualified the `std::println` disallowed-macro rule; the CLI's
// control subcommands print method results on stdout by design.
#![allow(clippy::print_stdout, clippy::print_stderr, clippy::disallowed_macros)]

mod cli_util;
mod cmd_backup;
mod cmd_ctl;
mod cmd_daemon;
mod cmd_daemon_flags;
mod cmd_deploy_card;
mod cmd_entity;
mod cmd_health;
mod cmd_incidents;
mod cmd_learn;
mod cmd_loop;
mod cmd_metrics;
mod cmd_oneshot;
mod cmd_run;
mod cmd_sandbox_exec;
mod cmd_soak;
mod cmd_sop;
mod cmd_store;
mod cmd_sys;
mod cmd_trace;
mod cmd_tui;
mod cmd_worker_exec;
mod daemon_log;
mod ndjson;

use crate::cli_util::fail;
use std::process::ExitCode;

const USAGE: &str = "usage: pii [--version] | pii -p \"<prompt>\" [...] | pii daemon [...] | pii sandbox-exec ... | pii graph {lint|show|run|resume|trace} ... | pii sop {compile|lint|show|run|resume} ... | pii store compact [--dir D] | pii memory {entity {add|show|list}|link|neighbours|compact|show|tiers|archive|restore} ... | pii incidents scan --root <dir> [--new] | pii incidents scan --source runtime --log <file> | pii deploy-card {render|approve|refuse} --card <file> | pii loop {emit|events} ... | pii learn {scan|list|show|verify|promote|promotions|revert|ledger} ...";

/// Renders the version banner; testable without spawning a process.
#[must_use]
pub fn version_line() -> String {
    format!("pii {}", env!("CARGO_PKG_VERSION"))
}

fn cmd_graph_lint(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            fail("graph", format!("read {path}: {e}"));
            return ExitCode::from(2);
        }
    };
    let reg = pii_server::lint_registry(std::path::Path::new("."), std::path::Path::new("."));
    let spec = match pii_graph::load_toml_with_registry(&src, &reg) {
        Ok(s) => s,
        Err(e) => {
            fail("graph", &e);
            return ExitCode::from(2);
        }
    };
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

/// `pii graph show <file.toml>` — validate then pipe the spec's human-readable
/// ASCII topology rendering to stdout (spec 10). Runs the same loader and the
/// same validator as [`cmd_graph_lint`] first, so a malformed graph is never
/// rendered (it is reported verbatim with its `V<n>` code instead). Purely
/// offline and deterministic: no daemon, no socket, no run state.
fn cmd_graph_show(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            fail("graph", format!("read {path}: {e}"));
            return ExitCode::from(2);
        }
    };
    // Same registry as lint/run so `show` accepts every graph those do,
    // including `agent`/`tool`/`route`/`gate` starter graphs.
    let reg = pii_server::lint_registry(std::path::Path::new("."), std::path::Path::new("."));
    let spec = match pii_graph::load_toml_with_registry(&src, &reg) {
        Ok(s) => s,
        Err(e) => {
            fail("graph", &e);
            return ExitCode::from(2);
        }
    };
    if let Err(errors) = pii_graph::validate(&spec, &reg) {
        for err in &errors {
            eprintln!("{err}");
        }
        return ExitCode::from(3);
    }
    print!("{}", pii_graph::render::render_graph(&spec));
    ExitCode::SUCCESS
}

// reason: one dispatch arm per subcommand; splitting it would only hide the list.
#[allow(clippy::too_many_lines)]
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
    if cmd == "-p" {
        let Some(prompt) = args.next().and_then(|a| a.into_string().ok()) else {
            eprintln!("usage: pii -p \"<prompt>\" [--json] [--model M] ...");
            return ExitCode::from(2);
        };
        return cmd_oneshot::cmd_oneshot(prompt, &mut args);
    }
    if cmd == "daemon" {
        return cmd_daemon::cmd_daemon(&mut args);
    }
    if cmd == "worker-exec" {
        return cmd_worker_exec::cmd_worker_exec();
    }
    if cmd == "sandbox-exec" {
        return cmd_sandbox_exec::cmd_sandbox_exec(&mut args);
    }
    if cmd == "tui" {
        return cmd_tui::cmd_tui(&mut args);
    }
    if cmd == "graph" {
        return cmd_graph(&mut args);
    }
    if cmd == "sop" {
        return cmd_sop::cmd_sop(&mut args);
    }
    if cmd == "net" {
        return cmd_ctl::cmd_net(&mut args);
    }
    if cmd == "worker" {
        return cmd_ctl::cmd_worker(&mut args);
    }
    if cmd == "memory" {
        return cmd_ctl::cmd_memory(&mut args);
    }
    if cmd == "approve" {
        return cmd_ctl::cmd_approve(&mut args);
    }
    if cmd == "heal" {
        return cmd_ctl::cmd_heal(&mut args);
    }
    if cmd == "cron" {
        return cmd_ctl::cmd_cron(&mut args);
    }
    if cmd == "soak" {
        return cmd_soak::cmd_soak(&mut args);
    }
    if cmd == "sys" {
        return cmd_sys::cmd_sys(&mut args);
    }
    if cmd == "health" {
        return cmd_health::cmd_health(&mut args);
    }
    if cmd == "metrics" {
        return cmd_metrics::cmd_metrics(&mut args);
    }
    if cmd == "backup" {
        return cmd_backup::cmd_backup(&mut args);
    }
    if cmd == "restore" {
        return cmd_backup::cmd_restore(&mut args);
    }
    if cmd == "store" {
        return cmd_store::cmd_store(&mut args);
    }
    if cmd == "learn" {
        return cmd_learn::cmd_learn(&mut args);
    }
    if cmd == "incidents" {
        return cmd_incidents::cmd_incidents(&mut args);
    }
    if cmd == "loop" {
        return cmd_loop::cmd_loop(&mut args);
    }
    if cmd == "deploy-card" {
        return cmd_deploy_card::cmd_deploy_card(&mut args);
    }
    if cmd == "run" {
        return cmd_sys::cmd_run_ctl(&mut args);
    }
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn cmd_graph(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(sub) = args.next() else {
        eprintln!("usage: pii graph {{lint|show|run|resume|trace}} ...");
        return ExitCode::from(2);
    };
    let sub = sub.to_str().map_or_else(|| "?".to_owned(), str::to_owned);
    // Consume the positional ONLY for the two file-driven subcommands — an
    // unconditional args.next() here would eat `graph run`'s <file.toml>.
    if sub == "lint" || sub == "show" {
        let Some(file_arg) = args.next() else {
            eprintln!("usage: pii graph {sub} <file.toml>");
            return ExitCode::from(2);
        };
        let file = file_arg.into_string().unwrap_or_default();
        return if sub == "lint" {
            cmd_graph_lint(&file)
        } else {
            cmd_graph_show(&file)
        };
    }
    match sub.as_str() {
        "run" => cmd_run::cmd_run(args),
        "resume" => cmd_run::cmd_resume(args),
        "trace" => cmd_trace::cmd_trace(args),
        _ => {
            eprintln!("usage: pii graph {{lint|show|run|resume|trace}} ...");
            ExitCode::from(2)
        }
    }
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
