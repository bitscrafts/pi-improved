//! Spec 322, end to end: the daemon writes its start mark before anything that can fail.
//!
//! A start that dies while opening the store must still leave a line in `pii.log`, and the runtime
//! scan must call it a lost start. Before this spec the first line appeared only after the socket was
//! bound, so a failure before that point was invisible in pii's own record.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads the log it writes.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

fn scratch(name: &str) -> std::path::PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_tmp")
        .join(format!("daemon-start-mark-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_daemon_logs_starting_before_it_can_fail() {
    let d = scratch("store-unwritable");
    // The store path is a directory: opening it as a database must fail, after the mark is written.
    let store = d.join("store-is-a-dir");
    std::fs::create_dir_all(&store).unwrap();
    let log = d.join("pii.log");
    let out = Command::new(env!("CARGO_BIN_EXE_pii"))
        .args([
            "daemon",
            "--foreground",
            "--store",
            store.to_str().unwrap(),
            "--workspace",
            d.to_str().unwrap(),
            "--log-file",
            log.to_str().unwrap(),
        ])
        .env("PII_CONFIG", d.join("pii.toml"))
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a store that cannot be opened must fail the start: {:?}",
        out.status
    );

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let first_kind = text
        .lines()
        .next()
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
        .unwrap_or_default();
    assert_eq!(
        first_kind, "daemon.starting",
        "the mark is the FIRST line of the log: {text}"
    );
    assert!(
        !text.contains("daemon.started"),
        "a start that never served must not claim it started: {text}"
    );
}
