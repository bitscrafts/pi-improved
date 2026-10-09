//! `pii graph run` / `pii graph resume` (spec F5.2–F5.6).

use crate::cli_util::fail;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use pii_abi::{PiiError, RunId, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, PathScope, TraceCtx};

use crate::cli_util::{next_value, open_store, usage_error};
use crate::ndjson::NdjsonObserver;

const RUN_USAGE: &str = "usage: pii graph run <file.toml> --db <path> [--run-id <id>] \
[--input <json>] [--allow-write <dir>]... [--no-validate] [--input defaults to {}]";
const RESUME_USAGE: &str = "usage: pii graph resume <run_id> --db <path> [--allow-write <dir>]...";

/// Shared flags for run/resume (spec F5.2/F5.4). `--input` defaults to `{}`.
struct Common {
    db: PathBuf,
    run_id: Option<RunId>,
    input: serde_json::Value,
    allow_write: Vec<PathBuf>,
    no_validate: bool,
}

impl Common {
    /// Parses the flag tail of `run`/`resume` (any order, each repeatable
    /// except `--db`; `--no-validate` only valid for `run`).
    ///
    /// # Errors
    /// Returns the usage exit code on an unknown flag, missing value, or
    /// invalid `--input` JSON.
    fn parse(
        args: &mut dyn Iterator<Item = std::ffi::OsString>,
        usage: &str,
        allow_novalidate: bool,
    ) -> Result<Self, ExitCode> {
        let mut db = None;
        let mut run_id = None;
        let mut input = serde_json::json!({});
        let mut allow_write = Vec::new();
        let mut no_validate = false;
        while let Some(arg) = args.next().and_then(|a| a.into_string().ok()) {
            match arg.as_str() {
                "--db" => db = Some(PathBuf::from(next_value(args, "--db", usage)?)),
                "--run-id" => run_id = Some(RunId::from(next_value(args, "--run-id", usage)?)),
                "--input" => {
                    let raw = next_value(args, "--input", usage)?;
                    input = serde_json::from_str(&raw)
                        .map_err(|_| usage_error("--input must be valid JSON", usage))?;
                }
                "--allow-write" => {
                    allow_write.push(PathBuf::from(next_value(args, "--allow-write", usage)?));
                }
                "--no-validate" if allow_novalidate => no_validate = true,
                other => return Err(usage_error(&format!("unknown argument '{other}'"), usage)),
            }
        }
        let Some(db) = db else {
            return Err(usage_error("missing --db <path>", usage));
        };
        Ok(Self {
            db,
            run_id,
            input,
            allow_write,
            no_validate,
        })
    }
}

/// `pii graph run <file.toml> ...` — load, validate, execute, journal.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_run(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(file) = args.next().and_then(|a| a.into_string().ok()) else {
        return usage_error("missing <file.toml>", RUN_USAGE);
    };
    let common = match Common::parse(args, RUN_USAGE, true) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Some(store) = open_store(&common.db) else {
        return ExitCode::from(2);
    };
    // F13 (opus review): use the server registry (built-ins + agent + tool)
    // so `graph run` accepts the same graphs `graph lint` accepts. The
    // builtin-only registry rejected starter graphs with unknown node kinds.
    let runtime_dir = common
        .db
        .parent()
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
    let reg = pii_server::lint_registry(std::path::Path::new("."), &runtime_dir);
    let spec = match load_and_validate(&file, &reg, common.no_validate) {
        Ok(spec) => spec,
        Err(code) => return code,
    };
    let run_id = match common.run_id.clone() {
        Some(id) => id,
        None => match store.next_run_id() {
            Ok(id) => id,
            Err(e) => {
                fail("graph", &e);
                return ExitCode::from(2);
            }
        },
    };
    let ctx = root_ctx(&common.allow_write, run_id.clone(), &spec.limits);
    let result = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        common.input,
        !common.no_validate,
    );
    report(result)
}

/// `pii graph resume <run_id> ...` — replay the journal, continue the loop.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_resume(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(run) = args.next().and_then(|a| a.into_string().ok()) else {
        return usage_error("missing <run_id>", RESUME_USAGE);
    };
    let common = match Common::parse(args, RESUME_USAGE, false) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let Some(store) = open_store(&common.db) else {
        return ExitCode::from(2);
    };
    // F13 (opus review): server registry so resume accepts agent/tool graphs.
    let runtime_dir = common
        .db
        .parent()
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
    let reg = pii_server::lint_registry(std::path::Path::new("."), &runtime_dir);
    let run_id = RunId::from(run);
    // Resume recovers the spec (and its limits) inside `resume()`; the root
    // budget here is unbounded so a continuation is not spuriously cut off.
    let ctx = root_ctx(
        &common.allow_write,
        run_id.clone(),
        &pii_graph::GraphLimits::default(),
    );
    let result = pii_graph::run::resume(&store, &reg, &ctx, &run_id, None);
    report(result)
}

/// Loads `file.toml` and validates it unless `--no-validate` (F5.6: exit 3).
fn load_and_validate(
    file: &str,
    reg: &pii_graph::NodeRegistry,
    no_validate: bool,
) -> Result<pii_graph::GraphSpec, ExitCode> {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => return Err(usage_error(&format!("read {file}: {e}"), RUN_USAGE)),
    };
    let spec = match pii_graph::load_toml(&src) {
        Ok(s) => s,
        Err(e) => return Err(usage_error(&format!("{e}"), RUN_USAGE)),
    };
    if !no_validate && let Err(errors) = pii_graph::validate(&spec, reg) {
        for e in &errors {
            eprintln!("{e}");
        }
        return Err(ExitCode::from(3));
    }
    Ok(spec)
}

/// Root capabilities per F5.4: `fs_read` allow = `[cwd]`; `fs_write` allow =
/// `[cwd/_tmp]` plus every `--allow-write <dir>`; all other bits clear. The
/// graph's declared caps are intersected per node inside the engine. The root
/// budget is built from the graph's `limits` (pass-1 §3.5: budget checked
/// before dispatch and after each node).
fn root_ctx(allow_write: &[PathBuf], run: RunId, limits: &pii_graph::GraphLimits) -> Ctx {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    root_ctx_in(&cwd, allow_write, run, limits)
}

/// [`root_ctx`] with the working directory supplied by the caller — the
/// testable seam (same CapSet/Budget construction, no process-global read).
fn root_ctx_in(
    cwd: &std::path::Path,
    allow_write: &[PathBuf],
    run: RunId,
    limits: &pii_graph::GraphLimits,
) -> Ctx {
    let mut fs_write = vec![cwd.join("_tmp")];
    // Canonicalise each --allow-write so the grant matches the canonical
    // target FsCap checks (opus review F15): a relative or symlinked grant
    // otherwise silently grants nothing. Create the dir first if absent so
    // dunce::canonicalize succeeds.
    for p in allow_write {
        let _ = std::fs::create_dir_all(p);
        let canon = dunce::canonicalize(p).unwrap_or_else(|_| p.clone());
        fs_write.push(canon);
    }
    let caps = CapSet {
        kinds: CapKind::FS_READ | CapKind::FS_WRITE | CapKind::PROC,
        fs_read: PathScope::new(vec![cwd.to_path_buf()]),
        fs_write: PathScope::new(fs_write),
        proc_argv0: ArgvScope::new(vec![String::from("/bin/sh")]),
        ..CapSet::default()
    };
    // Spec 103: with $PII_TRAJECTORY_DIR set, the run's journal and verbatim
    // terminal also go to the trajectory sink; unset, stdout is unchanged.
    let obs: Arc<dyn pii_core::Observer> = match pii_core::trajectory_sink(None) {
        Some(sink) => Arc::new(pii_core::MultiObserver::new(vec![
            Arc::new(pii_core::TrajectoryBridge::new(sink, &run, None)),
            Arc::new(NdjsonObserver),
        ])),
        None => Arc::new(NdjsonObserver),
    };
    Ctx::with_observer(
        caps,
        Budget::new(
            limits.max_tokens,
            limits.max_tool_calls,
            limits.max_micro_usd,
            limits.max_wall_ms,
        ),
        obs,
        TraceCtx { run, span: 0 },
    )
}

/// Prints the final `{"terminal":..}` line; maps to F5.3 exit codes.
fn report(result: Result<Terminal, PiiError>) -> ExitCode {
    match result {
        Ok(terminal) => {
            let body = serde_json::to_string(&terminal).unwrap_or_else(|_| "null".into());
            println!(r#"{{"terminal":{body}}}"#);
            match terminal {
                Terminal::Success { .. } => ExitCode::SUCCESS,
                Terminal::Failure { error } => {
                    fail("graph", &error);
                    ExitCode::from(1)
                }
                Terminal::Suspended { .. } => ExitCode::from(4),
            }
        }
        Err(e) => {
            fail("graph", &e);
            ExitCode::from(1)
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

    use std::ffi::OsString;
    use std::process::ExitCode;

    use pii_abi::{PiiError, ResumeKey, Terminal};

    use pii_abi::RunId;
    use pii_core::BudgetKind;
    use pii_core::CapKind;

    use super::{Common, report, root_ctx_in};

    fn os_args(argv: &[&str]) -> std::vec::IntoIter<OsString> {
        argv.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    // --- Common::parse ---------------------------------------------------

    #[test]
    fn common_parse_handles_valid_args() {
        let mut args = os_args(&[
            "--db",
            "/tmp/run.redb",
            "--run-id",
            "r1",
            "--input",
            r#"{"k":1}"#,
            "--allow-write",
            "/tmp/w",
            "--allow-write",
            "/tmp/w2",
            "--no-validate",
        ]);
        let c = Common::parse(&mut args, "USAGE", true).unwrap();
        assert_eq!(c.db, std::path::PathBuf::from("/tmp/run.redb"));
        assert_eq!(c.run_id.map(|r| r.to_string()), Some(String::from("r1")));
        assert_eq!(c.input, serde_json::json!({"k": 1}));
        assert_eq!(
            c.allow_write,
            vec![
                std::path::PathBuf::from("/tmp/w"),
                std::path::PathBuf::from("/tmp/w2")
            ]
        );
        assert!(c.no_validate);
    }

    #[test]
    fn common_parse_defaults_and_missing_db() {
        // --input defaults to {}; a missing --db is a usage error (2).
        let mut args = os_args(&[]);
        assert_eq!(
            Common::parse(&mut args, "USAGE", false).err().unwrap(),
            ExitCode::from(2)
        );
        let mut args = os_args(&["--db", "x.redb"]);
        let c = Common::parse(&mut args, "USAGE", false).unwrap();
        assert_eq!(c.input, serde_json::json!({}));
        assert!(!c.no_validate);
        assert!(c.run_id.is_none());
        assert!(c.allow_write.is_empty());
    }

    #[test]
    fn common_parse_rejects_unknown_flag_and_bad_json() {
        let mut args = os_args(&["--db", "x.redb", "--bogus"]);
        assert_eq!(
            Common::parse(&mut args, "USAGE", false).err().unwrap(),
            ExitCode::from(2)
        );
        let mut args = os_args(&["--db", "x.redb", "--input", "not-json"]);
        assert_eq!(
            Common::parse(&mut args, "USAGE", false).err().unwrap(),
            ExitCode::from(2)
        );
    }

    // --- root_ctx --------------------------------------------------------

    #[test]
    fn root_ctx_builds_expected_capset() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let grant = tmp.path().join("grant");
        let cwd = tmp.path().to_path_buf();
        let limits = pii_graph::GraphLimits {
            max_tokens: 123,
            max_tool_calls: 7,
            max_micro_usd: 99,
            ..pii_graph::GraphLimits::default()
        };
        let ctx = root_ctx_in(
            &cwd,
            std::slice::from_ref(&grant),
            RunId::from("r1"),
            &limits,
        );
        let caps = ctx.caps();
        assert_eq!(
            caps.kinds,
            CapKind::FS_READ | CapKind::FS_WRITE | CapKind::PROC
        );
        // fs_read allow = [cwd]; fs_write allow = [cwd/_tmp] + canonicalised
        // grants; proc argv0 = /bin/sh; other scopes empty.
        assert!(caps.fs_read.permits(&cwd));
        assert!(!caps.fs_read.permits(std::path::Path::new("/etc")));
        assert!(caps.fs_write.permits(&cwd.join("_tmp")));
        assert!(
            caps.fs_write
                .permits(&dunce::canonicalize(&grant).expect("canon"))
        );
        assert!(caps.proc_argv0.permits("/bin/sh"));
        assert!(!caps.net_hosts.permits("example.com"));
        // Budget mirrors the graph limits.
        assert_eq!(ctx.budget().remaining(BudgetKind::Tokens), 123);
        assert_eq!(ctx.budget().remaining(BudgetKind::ToolCalls), 7);
        assert_eq!(ctx.budget().remaining(BudgetKind::MicroUsd), 99);
        // Trace context carries the run id.
        assert_eq!(ctx.trace().run, RunId::from("r1"));
    }

    // --- report ----------------------------------------------------------

    #[test]
    fn report_maps_terminal_states_to_exit_codes() {
        assert_eq!(
            report(Ok(Terminal::Success {
                output: serde_json::json!({"ok": true})
            })),
            ExitCode::SUCCESS
        );
        assert_eq!(
            report(Ok(Terminal::Failure {
                error: String::from("boom")
            })),
            ExitCode::from(1)
        );
        assert_eq!(
            report(Ok(Terminal::Suspended {
                key: ResumeKey(String::from("k"))
            })),
            ExitCode::from(4)
        );
        assert_eq!(
            report(Err(PiiError::Invalid(String::from("bad")))),
            ExitCode::from(1)
        );
    }
}
