//! Spec 344: a second `pii daemon` whose durable store is held by a running daemon refuses at once
//! (exit 3, `cli.failure` with `store_locked`, the store path and "another daemon" on stderr)
//! instead of blocking silently on the store lock.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "daemon_guard/mod.rs"]
mod daemon_guard;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// One daemon's own dirs: workspace, runtime dir, state dir, log file.
struct Dirs {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    runtime: PathBuf,
    state: PathBuf,
    log: PathBuf,
    err: PathBuf,
    model: PathBuf,
}

fn dirs(prefix: &str) -> Dirs {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let root = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(scratch)
        .unwrap();
    let base = root.path().to_path_buf();
    let d = Dirs {
        workspace: base.join("ws"),
        runtime: base.join("run"),
        state: base.join("state"),
        log: base.join("daemon.log"),
        err: base.join("stderr.txt"),
        model: base.join("model.json"),
        _root: root,
    };
    for p in [&d.workspace, &d.runtime, &d.state] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(
        &d.model,
        r#"{"turns": [{"text": "ok", "tool_calls": [], "tokens_in": 1, "tokens_out": 1}]}"#,
    )
    .unwrap();
    d
}

fn start(d: &Dirs) -> daemon_guard::DaemonGuard {
    let stderr = std::fs::File::create(&d.err).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pii"));
    cmd.arg("daemon")
        .args(["--sandbox", "off", "--workspace"])
        .arg(&d.workspace)
        .arg("--log-file")
        .arg(&d.log)
        .env("PII_RUNTIME_DIR", &d.runtime)
        .env("PII_STATE_DIR", &d.state)
        .env("PII_MODEL", format!("scripted:{}", d.model.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    daemon_guard::spawn(&mut cmd)
}

fn wait_for_socket(d: &Dirs) {
    let until = Instant::now() + Duration::from_secs(10);
    while !d.runtime.join("control.sock").exists() {
        assert!(Instant::now() < until, "daemon never bound control.sock");
        thread::sleep(Duration::from_millis(50));
    }
}

/// Every `*.redb` under the state dir with its mtime, so a refused probe can be shown to leave them alone.
fn store_mtimes(state: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(state).unwrap().flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "redb") {
            out.push((
                p.clone(),
                std::fs::metadata(&p).unwrap().modified().unwrap(),
            ));
        }
    }
    out.sort();
    out
}

/// Waits up to 10 s for the second daemon to exit. `Err` carries its log: it hung, and is killed.
fn wait_refusal(
    b: &mut daemon_guard::DaemonGuard,
    log: &Path,
) -> Result<std::process::ExitStatus, String> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = b.child_mut().try_wait().unwrap() {
            return Ok(status);
        }
        if Instant::now() >= until {
            b.kill_now();
            return Err(std::fs::read_to_string(log).unwrap_or_default());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_second_daemon_refuses_and_says_which_store() {
    let a_dirs = dirs("sd-a");
    let mut a = start(&a_dirs);
    wait_for_socket(&a_dirs);

    let mut b_dirs = dirs("sd-b");
    // One durable store per user: B shares A's state dir, and keeps its own workspace and runtime dir.
    b_dirs.state.clone_from(&a_dirs.state);
    let mut b = start(&b_dirs);
    let status = wait_refusal(&mut b, &b_dirs.log).expect("second daemon hung on the store lock");

    let stderr = std::fs::read_to_string(&b_dirs.err).unwrap();
    assert_eq!(status.code(), Some(3), "stderr: {stderr}");
    assert!(
        stderr.contains(".redb"),
        "stderr must name the store path: {stderr}"
    );
    assert!(
        a.child_mut().try_wait().unwrap().is_none(),
        "daemon A must still run"
    );
}

#[test]
fn the_refusal_names_the_running_daemon() {
    let a_dirs = dirs("sd-a");
    let _a = start(&a_dirs);
    wait_for_socket(&a_dirs);

    let mut b_dirs = dirs("sd-b");
    // One durable store per user: B shares A's state dir, and keeps its own workspace and runtime dir.
    b_dirs.state.clone_from(&a_dirs.state);
    let mut b = start(&b_dirs);
    wait_refusal(&mut b, &b_dirs.log).expect("second daemon hung on the store lock");

    let stderr = std::fs::read_to_string(&b_dirs.err).unwrap();
    assert!(
        stderr.contains("another daemon"),
        "refusal must say another daemon holds the store: {stderr}"
    );
    let log = std::fs::read_to_string(&b_dirs.log).unwrap_or_default();
    assert!(
        log.contains("cli.failure") && log.contains("store_locked"),
        "log must carry cli.failure with reason store_locked: {log}"
    );
}

#[test]
fn control_one_daemon_still_starts() {
    let a_dirs = dirs("sd-a");
    let _a = start(&a_dirs);
    wait_for_socket(&a_dirs);

    // The socket appears before every startup line is written (the boot retention sweep runs
    // first, spec 376 R4), so wait for the evidence instead of assuming the order.
    let mut log = String::new();
    for _ in 0..40 {
        log = std::fs::read_to_string(&a_dirs.log).unwrap_or_default();
        if log.contains("daemon.started") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(log.contains("daemon.started"), "log: {log}");
}

#[test]
fn control_a_refused_start_disturbs_nothing() {
    let a_dirs = dirs("sd-a");
    let _a = start(&a_dirs);
    wait_for_socket(&a_dirs);
    let before = store_mtimes(&a_dirs.state);

    let mut b_dirs = dirs("sd-b");
    // One durable store per user: B shares A's state dir, and keeps its own workspace and runtime dir.
    b_dirs.state.clone_from(&a_dirs.state);
    let mut b = start(&b_dirs);
    wait_refusal(&mut b, &b_dirs.log).expect("second daemon hung on the store lock");

    // Through `pii_uds::client_connect`, the one sanctioned client (net-check), not a raw `UnixStream`: a test that
    // proves the daemon answers must reach it the way the product does.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the client");
    let answered = rt.block_on(async {
        pii_uds::client_connect(&a_dirs.runtime.join("control.sock"))
            .await
            .is_ok()
    });
    assert!(answered, "daemon A's socket must still answer");
    assert_eq!(
        store_mtimes(&a_dirs.state),
        before,
        "a refused start must not touch A's stores"
    );
}
