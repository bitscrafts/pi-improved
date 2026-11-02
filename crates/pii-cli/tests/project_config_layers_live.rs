//! The project-local `pii.toml` is layering the DAEMON actually performs at start, not only a
//! function with a passing unit test: a `<workspace>/.pii/pii.toml` model wins over the global
//! file's, an absent project file leaves the global in force, and a global-only setting in the
//! project file stops the start by name.
//!
//! Driven through the built `pii` binary and the daemon's own `daemon.started` event, because
//! `Config::merge_project`'s own suite (`pii-server/tests/project_config_layers.rs`) proves the
//! merge and not that anything calls it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the tests run the built `pii` binary.
#![allow(clippy::disallowed_methods)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use daemon_guard::DaemonGuard;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The scripted fixtures the daemon can start on without a network model server.
fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/phase2/{name}"));
    format!("scripted:{}", p.display())
}

fn scratch(prefix: &str) -> tempfile::TempDir {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&dir).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(dir)
        .unwrap()
}

/// Starts the daemon against `global` config + `workspace`, waits for its `daemon.started` line
/// (or for it to exit), then stops it. Returns the log text.
fn start_and_read_log(global: &Path, workspace: &Path, log: &Path) -> String {
    let mut child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(workspace)
            .arg("--log-file")
            .arg(log)
            .env("PII_RUNTIME_DIR", workspace)
            .env("PII_CONFIG", global)
            .env_remove("PII_MODEL")
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(20);
    let mut text = String::new();
    while Instant::now() < end {
        text = std::fs::read_to_string(log).unwrap_or_default();
        if text.contains("daemon.started") || child.child_mut().try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill_now();
    text
}

/// Starts the daemon and LEAVES IT RUNNING: `sys.reload` needs a live control socket, so these
/// rows cannot use the start-then-stop harness above. The guard kills it on every exit path,
/// including a panicking assertion.
fn start_live_daemon(global: &Path, workspace: &Path, log: &Path) -> DaemonGuard {
    let mut child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(workspace)
            .arg("--log-file")
            .arg(log)
            .env("PII_RUNTIME_DIR", workspace)
            .env("PII_CONFIG", global)
            .env_remove("PII_MODEL")
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        if text.contains("daemon.started") || child.child_mut().try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child
}

/// One `pii` control verb against the live daemon's socket: `(code, stdout, stderr)`.
fn ctl(workspace: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", workspace)
        .env_remove("PII_MODEL")
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn the_workspace_project_file_wins_over_the_global_file() {
    let dir = scratch("pcfg-");
    let global = dir.path().join("global.toml");
    let project_model = fixture("selfchange.json");
    let global_model = fixture("hello.json");
    std::fs::write(&global, format!("[core]\nmodel = \"{global_model}\"\n")).unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(workspace.join(".pii")).unwrap();
    std::fs::write(
        workspace.join(".pii").join("pii.toml"),
        format!("[core]\nmodel = \"{project_model}\"\n"),
    )
    .unwrap();
    let log = dir.path().join("daemon.log");

    let text = start_and_read_log(&global, &workspace, &log);

    assert!(
        text.contains(&project_model),
        "the project file's model must be the one in force; log: {text}"
    );
    assert!(
        !text.contains(&global_model),
        "the global model must NOT be the one in force; log: {text}"
    );
}

#[test]
fn without_a_project_file_the_global_file_still_applies() {
    // The near-miss control for the test above: same harness, no project file. If the daemon
    // always took some other path, that test would pass for the wrong reason.
    let dir = scratch("pcfg-");
    let global = dir.path().join("global.toml");
    let global_model = fixture("hello.json");
    std::fs::write(&global, format!("[core]\nmodel = \"{global_model}\"\n")).unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let log = dir.path().join("daemon.log");

    let text = start_and_read_log(&global, &workspace, &log);

    assert!(
        text.contains(&global_model),
        "an absent project file leaves the global model in force; log: {text}"
    );
}

#[test]
fn a_global_only_setting_in_the_project_file_stops_the_start() {
    let dir = scratch("pcfg-");
    let global = dir.path().join("global.toml");
    std::fs::write(
        &global,
        format!("[core]\nmodel = \"{}\"\n", fixture("hello.json")),
    )
    .unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(workspace.join(".pii")).unwrap();
    // runtime_dir decides where the daemon writes: a repository must not move it.
    std::fs::write(
        workspace.join(".pii").join("pii.toml"),
        "[core]\nruntime_dir = \"/tmp/elsewhere\"\n",
    )
    .unwrap();
    let log: PathBuf = dir.path().join("daemon.log");

    // A bounded wait, not `.output()`: on a tree where the refusal does not happen the daemon
    // starts and serves, so an unbounded wait would hang the suite instead of failing it.
    let mut child = DaemonGuard::new(
        Command::new(env!("CARGO_BIN_EXE_pii"))
            .arg("daemon")
            .args(["--sandbox", "off", "--workspace"])
            .arg(&workspace)
            .arg("--log-file")
            .arg(&log)
            .env("PII_RUNTIME_DIR", &workspace)
            .env("PII_CONFIG", &global)
            .env_remove("PII_MODEL")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut status = None;
    while Instant::now() < deadline {
        if let Some(s) = child.child_mut().try_wait().unwrap() {
            status = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if status.is_none() {
        child.kill_now();
        panic!(
            "the daemon started despite a global-only project setting (log: {})",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
    }
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert_eq!(
        status.unwrap().code(),
        Some(2),
        "a refused project setting is a fatal start; stderr: {stderr}"
    );
    assert!(
        stderr.contains("global-only") && stderr.contains("runtime_dir"),
        "the refusal names the setting; stderr: {stderr}"
    );
    assert!(
        !std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("daemon.started"),
        "the daemon must not have started"
    );
}

/// The reload rows (spec 235): the SAME layers must be in force after `pii sys reload`, because
/// one daemon that answers two different "which config is in force" questions silently reverts
/// the project's model, llama host and tiers on the first reload.
#[test]
fn a_global_only_setting_added_after_start_is_refused_by_reload() {
    let dir = scratch("pcfg-");
    let global = dir.path().join("global.toml");
    std::fs::write(
        &global,
        format!("[core]\nmodel = \"{}\"\n", fixture("hello.json")),
    )
    .unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(workspace.join(".pii")).unwrap();
    let project = workspace.join(".pii").join("pii.toml");
    std::fs::write(
        &project,
        format!("[core]\nmodel = \"{}\"\n", fixture("selfchange.json")),
    )
    .unwrap();
    let log = dir.path().join("daemon.log");
    let _daemon = start_live_daemon(&global, &workspace, &log);

    // A repository may not move where the daemon writes. `runtime_dir` is global-only, so the
    // layer that refuses it must be the one reload applies — the global file alone would never
    // look at this file and the reload would answer `workers_reloaded` instead.
    std::fs::write(&project, "[core]\nruntime_dir = \"/tmp/elsewhere\"\n").unwrap();

    let (code, stdout, stderr) = ctl(&workspace, &["sys", "reload"]);

    assert_eq!(
        code,
        Some(2),
        "a refused project setting fails the reload; stdout: {stdout}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("global-only") && stderr.contains("runtime_dir"),
        "the refusal names the setting; stderr: {stderr}"
    );
    assert!(
        !stdout.contains("workers_reloaded"),
        "nothing from a refused file is applied; stdout: {stdout}"
    );
    // The refusal must not take the daemon down with it.
    let (status_code, status_out, status_err) = ctl(&workspace, &["sys", "status"]);
    assert_eq!(
        status_code,
        Some(0),
        "the daemon keeps serving after a refused reload; stdout: {status_out}; stderr: {status_err}"
    );
}

#[test]
fn a_valid_project_file_reloads_instead_of_erroring() {
    let dir = scratch("pcfg-");
    let global = dir.path().join("global.toml");
    let project_model = fixture("selfchange.json");
    std::fs::write(
        &global,
        format!("[core]\nmodel = \"{}\"\n", fixture("hello.json")),
    )
    .unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(workspace.join(".pii")).unwrap();
    std::fs::write(
        workspace.join(".pii").join("pii.toml"),
        format!("[core]\nmodel = \"{project_model}\"\n"),
    )
    .unwrap();
    let log = dir.path().join("daemon.log");
    let _daemon = start_live_daemon(&global, &workspace, &log);

    let (code, stdout, stderr) = ctl(&workspace, &["sys", "reload"]);

    assert_eq!(
        code,
        Some(0),
        "a valid project file reloads; stdout: {stdout}; stderr: {stderr}"
    );
    assert!(
        stdout.contains("workers_reloaded"),
        "the operator's view is unchanged; stdout: {stdout}"
    );
}

#[test]
fn an_absent_project_file_still_reloads_the_global() {
    // The near-miss control for the two rows above: same harness, no project file at all. If the
    // reload always read some project path, the refusal row would pass for the wrong reason.
    let dir = scratch("pcfg-");
    let global = dir.path().join("global.toml");
    std::fs::write(
        &global,
        format!("[core]\nmodel = \"{}\"\n", fixture("hello.json")),
    )
    .unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let log = dir.path().join("daemon.log");
    let _daemon = start_live_daemon(&global, &workspace, &log);

    let (code, stdout, stderr) = ctl(&workspace, &["sys", "reload"]);

    assert_eq!(
        code,
        Some(0),
        "without a project file the global config applies and the reload succeeds; \
         stdout: {stdout}; stderr: {stderr}"
    );
    assert!(
        stdout.contains("workers_reloaded"),
        "the answer shape does not depend on the layer; stdout: {stdout}"
    );
}
