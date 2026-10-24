//! Spec 257 R5: a malformed `--input` is an error before the daemon is contacted, not a silent `{}`. Valid JSON and no flag
//! still reach the daemon path (the control: with no daemon they fail with exit 1, not exit 2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout and stderr.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Output};

/// Runs `pii run start g` with `extra` after it, against an empty runtime dir: there is no daemon socket.
fn pii_start(runtime: &Path, extra: &[&str]) -> Output {
    let home = runtime.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .env("HOME", &home)
        .env("PII_RUNTIME_DIR", runtime)
        .args(["run", "start", "g"])
        .args(extra)
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

#[test]
fn a_malformed_input_exits_2_without_a_daemon() {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "run-bad-input-").unwrap();
    let out = pii_start(root.path(), &["--input", "{bad"]);
    assert_eq!(
        code(&out),
        2,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--input must be valid JSON"), "stderr: {err}");
    assert!(
        !err.contains("control.sock"),
        "the daemon was not contacted: {err}"
    );
}

#[test]
fn a_missing_input_value_exits_2() {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "run-bad-input-").unwrap();
    let out = pii_start(root.path(), &["--input"]);
    assert_eq!(
        code(&out),
        2,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--input needs a value"), "stderr: {err}");
}

/// The daemon path is the one that names its socket when no daemon answers. The exit code does not tell the two paths
/// apart: with no daemon, `run start` exits 2 with or without `--input`. So the control reads stderr.
#[test]
fn valid_input_and_no_input_still_reach_the_daemon_path() {
    let root =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "run-bad-input-").unwrap();
    for out in [
        pii_start(root.path(), &["--input", "{}"]),
        pii_start(root.path(), &[]),
    ] {
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("--input"), "not a usage error: {err}");
        assert!(
            err.contains("control.sock"),
            "reached the daemon path: {err}"
        );
    }
}
