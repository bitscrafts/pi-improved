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
