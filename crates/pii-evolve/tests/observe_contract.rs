//! Spec 101 R3: a failed boss launch and the cycle start reach a
//! `pii-observe` observer by their registered names.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_evolve::driver::{BossLaunch, run_cycle_observed};
use pii_observe::{Recorder, names};

#[test]
fn launch_failure_and_cycle_start_are_reported_by_name() {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("evolve-observe-")
        .tempdir_in(&root)
        .unwrap();
    // No daemon is listening on this socket path.
    let cfg = BossLaunch::new(dir.path().join("absent.sock"), dir.path().to_path_buf());
    let rec = Recorder::new();
    assert!(run_cycle_observed(&cfg, &rec).is_err());

    let started = rec.events_named(names::EVOLVE_CYCLE_STARTED);
    assert_eq!(started.len(), 1, "{:?}", rec.events());
    assert_eq!(started[0].fields.get("cycle"), Some(&serde_json::json!(1)));
    let failed = rec.events_named(names::EVOLVE_LAUNCH_FAILED);
    assert_eq!(failed.len(), 1, "{:?}", rec.events());
    assert!(rec.events_named(names::EVOLVE_CYCLE_FINISHED).is_empty());
}

/// Spec 130 R3/R4: a missing socket is reported by `pii_uds` with the endpoint,
/// and the returned error still names the path.
#[test]
fn launcher_reports_a_missing_socket_with_the_path() {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("evolve-missing-")
        .tempdir_in(&root)
        .unwrap();
    let sock = dir.path().join("absent.sock");
    let cfg = BossLaunch::new(sock.clone(), dir.path().to_path_buf());
    let rec = Recorder::new();
    let err = run_cycle_observed(&cfg, &rec).unwrap_err();
    assert!(
        err.to_string().contains(&sock.display().to_string()),
        "error must name the socket path: {err}"
    );
    let failed = rec.events_named(names::UDS_CONNECT_FAILED);
    assert_eq!(failed.len(), 1, "{:?}", rec.events());
    assert_eq!(
        failed[0].fields.get("endpoint").and_then(|v| v.as_str()),
        Some(sock.display().to_string().as_str())
    );
}
