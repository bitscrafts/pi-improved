//! Spec 107 R1: the daemon is confined unless the operator opts out. Where
//! confinement cannot be enforced, the default start FAILS (exit 4); only an
//! explicit `--sandbox off` gets past the sandbox step.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the tests run the built `pii` binary.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Output};

fn daemon(extra: &[&str], force_unsupported: bool) -> Output {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("dsbx-")
        .tempdir_in(scratch)
        .unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_pii"));
    c.arg("daemon")
        .args(extra)
        .arg("--workspace")
        .arg(dir.path())
        .env("PII_RUNTIME_DIR", dir.path())
        // The state dir too, and for the same reason every other daemon-spawning test needs it (spec 344): without
        // it the daemon resolves the durable stores from $HOME and opens the OPERATOR's `channel.redb`, which a
        // long-running daemon already holds — the start then dies on the store lock instead of on the assertion
        // under test.
        .env("PII_STATE_DIR", dir.path())
        .env("PII_CONFIG", dir.path().join("absent.toml"))
        .env_remove("PII_MODEL");
    if force_unsupported {
        c.env("PII_SANDBOX_FORCE_UNSUPPORTED", "1");
    }
    c.output().unwrap()
}

#[test]
fn default_start_is_refused_when_confinement_cannot_be_enforced() {
    let o = daemon(&[], true);
    assert_eq!(
        o.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn explicit_off_gets_past_the_sandbox_step() {
    // Exit 5 = "no model configured", the step AFTER the sandbox step.
    let o = daemon(&["--sandbox", "off"], true);
    assert_eq!(
        o.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn explicit_off_is_reported_loudly_on_stderr() {
    let o = daemon(&["--sandbox", "off"], false);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("UNCONFINED"), "stderr: {err}");
}

#[test]
fn default_start_confines_and_proceeds_past_the_sandbox_step() {
    // Landlock is available here: the default applies it and reaches the
    // model check (exit 5), proving the workspace profile really was applied
    // without breaking the startup reads (config, runtime dir).
    let o = daemon(&[], false);
    assert_eq!(
        o.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
