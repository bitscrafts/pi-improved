//! `pii` command-line entry point (spec F5).

#![forbid(unsafe_code)]
// reason: the CLI must write to stdout/stderr directly (spec F5.5).
// spec 67 qualified the `std::println` disallowed-macro rule; the CLI's
// control subcommands print method results on stdout by design.
#![allow(clippy::print_stdout, clippy::print_stderr, clippy::disallowed_macros)]

#[macro_use]
mod cli_util;
mod cmd_backup;
mod cmd_ctl;
mod cmd_daemon;
mod cmd_daemon_flags;
mod cmd_deploy_card;
mod cmd_disk;
mod cmd_entity;
mod cmd_graph_installed;
mod cmd_health;
mod cmd_incidents;
mod cmd_index;
mod cmd_index_docs;
mod cmd_lane;
mod cmd_learn;
mod cmd_loop;
mod cmd_metrics;
mod cmd_mqtt;
mod cmd_oneshot;
mod cmd_project;
mod cmd_queue;
mod cmd_run;
mod cmd_sandbox_exec;
mod cmd_skill;
mod cmd_soak;
mod cmd_sop;
mod cmd_store;
mod cmd_sys;
mod cmd_trace;
mod cmd_treat;
mod cmd_tui;
mod cmd_worker_exec;
mod daemon_log;
mod graph_dirs;
mod ndjson;

use crate::cli_util::fail;
use std::process::ExitCode;

const USAGE: &str = "usage: pii [--version] | pii -p \"<prompt>\" [...] | pii daemon [...] | pii sandbox-exec ... | pii graph {lint|show|run|resume|trace|list|explain|edit} ... | pii sop {compile|lint|show|run|resume} ... | pii store compact [--dir D] | pii memory {entity {add|show|list}|learn|link|neighbours|compact|show|tiers|archive|restore} ... | pii skill {list|show|explain|edit} ... | pii project init <dir> | pii index {code [--root <dir>] [--max N]|docs [--root <dir>] [--kind spec|handoff|ops|all]} | pii incidents scan --root <dir> [--new] | pii incidents scan --source runtime --log <file> | pii deploy-card {render|approve|refuse} --card <file> | pii loop {emit|events} ... | pii learn {scan|list|show|verify|promote|promotions|revert|ledger} ...";

/// Renders the version banner; testable without spawning a process.
#[must_use]
pub fn version_line() -> String {
    format!("pii {}", env!("CARGO_PKG_VERSION"))
}

/// `pii graph {lint|check} <file.toml>` — static validation (spec 207 names it
/// `check`). A host graph is woven from its `fragments/` directory first (spec
/// 212), and every inlined node is printed with its surface, order and origin. A
/// fragment file is checked on its own contract instead.
fn cmd_graph_lint(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            fail("graph", format!("read {path}: {e}"));
            return ExitCode::from(2);
        }
    };
    let reg = pii_server::lint_registry(std::path::Path::new("."), std::path::Path::new("."));
    let is_fragment = pii_graph::fragment::declares_fragment(&src);
    let (spec, inlined) = if is_fragment {
        match pii_graph::fragment::check_fragment(&src, &reg) {
            Ok(fragment) => (fragment.spec, Vec::new()),
            Err(e) => {
                fail("graph", &e);
                return ExitCode::from(2);
            }
        }
    } else {
        let file = std::path::Path::new(path);
        let dirs = graph_dirs::dirs_for(file, Some(graph_dirs::default_store().as_path()));
        match pii_graph::fragment::load_host_with_graphs(
            &src,
            &graph_dirs::fragments_for(file),
            &dirs,
            &reg,
        ) {
            Ok(woven) => (woven.spec, woven.sequence),
            Err(e) => {
                fail("graph", &e);
                return ExitCode::from(2);
            }
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
            for i in &inlined {
                println!(
                    "  {:<10} order {:>3}  {}  woven-from: {}  hook: {}",
                    i.surface, i.order, i.node, i.origin, i.attach
                );
            }
            if is_fragment {
                for n in &spec.nodes {
                    println!("  fragment node  {}  kind {}", n.id, n.kind);
                }
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
    let file = std::path::Path::new(path);
    let dirs = graph_dirs::dirs_for(file, Some(graph_dirs::default_store().as_path()));
    let spec = match pii_graph::fragment::load_host_with_graphs(
        &src,
        &graph_dirs::fragments_for(file),
        &dirs,
        &reg,
    ) {
        Ok(woven) => woven.spec,
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
    if cmd == "index" {
        return cmd_index::cmd_index(&mut args);
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
    if cmd == "treat" {
        return cmd_treat::cmd_treat(&mut args);
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
    if cmd == "disk" {
        return cmd_disk::cmd_disk(&mut args);
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
    if cmd == "mqtt" {
        return cmd_mqtt::cmd_mqtt(&mut args);
    }
    if cmd == "incidents" {
        return cmd_incidents::cmd_incidents(&mut args);
    }
    if cmd == "loop" {
        return cmd_loop::cmd_loop(&mut args);
    }
    if cmd == "skill" {
        return cmd_skill::cmd_skill(&mut args);
    }
    if cmd == "project" {
        return cmd_project::cmd_project(&mut args);
    }
    if cmd == "deploy-card" {
        return cmd_deploy_card::cmd_deploy_card(&mut args);
    }
    if cmd == "lane" {
        return cmd_lane::cmd_lane(&mut args);
    }
    if cmd == "queue" {
        return cmd_queue::cmd_queue(&mut args);
    }
    if cmd == "run" {
        return cmd_sys::cmd_run_ctl(&mut args);
    }
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn cmd_graph(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(sub) = args.next() else {
        eprintln!("usage: pii graph {{lint|show|run|resume|trace|list|explain|edit}} ...");
        return ExitCode::from(2);
    };
    let sub = sub.to_str().map_or_else(|| "?".to_owned(), str::to_owned);
    // Consume the positional ONLY for the two file-driven subcommands — an
    // unconditional args.next() here would eat `graph run`'s <file.toml>.
    // `check` is the spec-207 name for `lint`: the same static validation.
    if sub == "lint" || sub == "check" || sub == "show" {
        let Some(file_arg) = args.next() else {
            eprintln!("usage: pii graph {sub} <file.toml>");
            return ExitCode::from(2);
        };
        let file = file_arg.into_string().unwrap_or_default();
        if sub == "show" && cmd_graph_installed::is_installed_name(&file) {
            return cmd_graph_installed::cmd_show(&file, args);
        }
        return if sub == "show" {
            cmd_graph_show(&file)
        } else {
            cmd_graph_lint(&file)
        };
    }
    match sub.as_str() {
        "run" => cmd_run::cmd_run(args),
        "resume" => cmd_run::cmd_resume(args),
        "trace" => cmd_trace::cmd_trace(args),
        "list" => cmd_graph_installed::cmd_list(args),
        "explain" => cmd_graph_installed::cmd_explain(args),
        "edit" => cmd_graph_installed::cmd_edit(args),
        _ => {
            eprintln!("usage: pii graph {{lint|show|run|resume|trace|list|explain|edit}} ...");
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
