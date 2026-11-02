//! Spec 339 R3: `pii health` from a plain shell (no `PII_RUNTIME_DIR`) names the
//! socket it tried and the service that creates it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary with a scrubbed environment.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Stdio};

fn scratch() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("health-no-env-")
        .tempdir_in(root)
        .unwrap()
}

/// `pii health` with no `PII_RUNTIME_DIR`, a home with no config, and a runtime root where nothing listens.
///
/// WHY `XDG_RUNTIME_DIR` IS SET TO THE SCRATCH DIR: removing it made the client fall back to the machine's real
/// default, where THIS CONTAINER'S dev daemon legitimately listens — so the test measured a version-drift answer
/// (rc 3) instead of "no daemon answered" (rc 2). The property under test is "no `PII_RUNTIME_DIR`, and the message
/// names where it looked", not "this machine has no daemon".
fn plain_shell_health(home: &Path) -> std::process::Output {
    // The container's dev daemon listens at the uid default (`/run/user/0/pii`), so "no daemon answers" has to be
    // arranged, not assumed: a config that pins `[core] runtime_dir` to this scratch directory is the supported
    // precedence for that, and it keeps `PII_RUNTIME_DIR` out of the environment — the property this test is about.
    let run = home.join("run");
    std::fs::create_dir_all(&run).unwrap();
    let cfg = home.join("pii.toml");
    std::fs::write(
        &cfg,
        format!("[core]\nruntime_dir = \"{}\"\n", run.display()),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("health")
        .env_remove("PII_RUNTIME_DIR")
        .env("XDG_RUNTIME_DIR", home)
        .env("HOME", home)
        .env("PII_CONFIG", &cfg)
        .env("PII_CONNECT_TIMEOUT_MS", "400")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn a_missing_socket_names_the_service() {
    let dir = scratch();
    let out = plain_shell_health(dir.path());
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("error: no daemon answered"), "{err}");
    assert!(err.contains("systemctl --user status pii"), "{err}");
}

#[test]
fn a_missing_socket_names_the_path_it_tried() {
    let dir = scratch();
    let out = plain_shell_health(dir.path());
    let err = String::from_utf8_lossy(&out.stderr);
    // the path it tried must be under the runtime root the test gave it, and must name the socket itself
    assert!(err.contains("control.sock"), "{err}");
    assert!(err.contains(&dir.path().display().to_string()), "{err}");
    assert!(!err.contains("is `pii daemon` running?"), "{err}");
}
