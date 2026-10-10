//! `pii sop {compile|lint|show|run|resume}` (spec 113): a declarative SOP
//! compiled into the existing graph. `run`/`resume` execute that graph through
//! the same engine and journal as `pii graph run`/`resume`; no second runner.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use pii_abi::RunId;

use crate::cli_util::{fail, open_store, usage_error};
use crate::cmd_run::{Common, report, root_ctx};

const USAGE: &str = "usage: pii sop compile <file.sop.toml> | lint <file.sop.toml> | show <file.sop.toml> \
| run <file.sop.toml> --db <path> [--run-id <id>] [--input <json>] [--allow-write <dir>]... \
| resume <run_id> --db <path> [--allow-write <dir>]...";
const RUN_USAGE: &str = "usage: pii sop run <file.sop.toml> --db <path> [--run-id <id>] \
[--input <json>] [--allow-write <dir>]...";
const RESUME_USAGE: &str = "usage: pii sop resume <run_id> --db <path> [--allow-write <dir>]...";

/// `pii sop ...` dispatcher.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_sop(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(sub) = args.next().and_then(|a| a.into_string().ok()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let Some(file) = args.next().and_then(|a| a.into_string().ok()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match sub.as_str() {
        "compile" => with_graph(&file, |_, text| {
            print!("{text}");
            ExitCode::SUCCESS
        }),
        "lint" => with_graph(&file, |_, _| {
            println!("{file}: OK");
            ExitCode::SUCCESS
        }),
        "show" => with_graph(&file, |spec, _| {
            print!("{}", pii_graph::render::render_graph(spec));
            ExitCode::SUCCESS
        }),
        "run" => cmd_run(&file, args),
        "resume" => cmd_resume(&file, args),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The server registry whose `sop` nodes report their events on stderr.
fn registry(runtime_dir: &std::path::Path) -> pii_graph::NodeRegistry {
    pii_server::lint_registry_with_log(
        std::path::Path::new("."),
        runtime_dir,
        Arc::new(pii_observe::StderrObserver),
    )
}

/// Loads and lints the SOP, compiles it, validates the compiled graph with the
/// real registry, then hands the spec and the graph text to `then`. Exit 2 =
/// unreadable file, 3 = the SOP or its graph was refused (one line each).
fn with_graph(file: &str, then: impl FnOnce(&pii_graph::GraphSpec, &str) -> ExitCode) -> ExitCode {
    match compiled(file, &registry(std::path::Path::new("."))) {
        Ok((spec, text)) => then(&spec, &text),
        Err(code) => code,
    }
}

fn compiled(
    file: &str,
    reg: &pii_graph::NodeRegistry,
) -> Result<(pii_graph::GraphSpec, String), ExitCode> {
    let src = std::fs::read_to_string(file).map_err(|e| {
        fail("sop", format!("read {file}: {e}"));
        ExitCode::from(2)
    })?;
    let sop = pii_graph::sop::load(&src).map_err(|errors| {
        for e in &errors {
            eprintln!("{file}: {e}");
        }
        ExitCode::from(3)
    })?;
    let text = pii_graph::sop::compile(&sop);
    let spec = pii_graph::load_toml_with_registry(&text, reg).map_err(|e| {
        fail("sop", &e);
        ExitCode::from(3)
    })?;
    if let Err(errors) = pii_graph::validate(&spec, reg) {
        for e in &errors {
            eprintln!("{file}: compiled graph refused: {e}");
        }
        return Err(ExitCode::from(3));
    }
    Ok((spec, text))
}

fn runtime_dir(db: &std::path::Path) -> PathBuf {
    db.parent()
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf)
}

fn cmd_run(file: &str, args: &mut std::env::ArgsOs) -> ExitCode {
    let common = match Common::parse(args, RUN_USAGE, false) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Some(store) = open_store(&common.db) else {
        return ExitCode::from(2);
    };
    let reg = registry(&runtime_dir(&common.db));
    let (spec, _) = match compiled(file, &reg) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let run_id = match common.run_id.clone() {
        Some(id) => id,
        None => match store.next_run_id() {
            Ok(id) => id,
            Err(e) => {
                fail("sop", &e);
                return ExitCode::from(2);
            }
        },
    };
    let ctx = root_ctx(&common.allow_write, run_id.clone(), &spec.limits);
    report(pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        common.input,
        true,
    ))
}

fn cmd_resume(run: &str, args: &mut std::env::ArgsOs) -> ExitCode {
    let common = match Common::parse(args, RESUME_USAGE, false) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Some(store) = open_store(&common.db) else {
        return ExitCode::from(2);
    };
    if run.is_empty() {
        return usage_error("missing <run_id>", RESUME_USAGE);
    }
    let reg = registry(&runtime_dir(&common.db));
    let run_id = RunId::from(run);
    let ctx = root_ctx(
        &common.allow_write,
        run_id.clone(),
        &pii_graph::GraphLimits::default(),
    );
    report(pii_graph::run::resume(&store, &reg, &ctx, &run_id, None))
}
