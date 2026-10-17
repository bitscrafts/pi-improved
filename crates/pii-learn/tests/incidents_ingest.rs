//! Spec 166: an opened incident lives in the store, with an id.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::IncidentConfig;
use pii_learn::incident_runtime::scan_runtime_log;
use pii_learn::incident_store::{ingest, show};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn files_in(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("incidents")).map_or(0, Iterator::count)
}

#[test]
fn an_opened_incident_is_stored_with_an_id() {
    let scan = scan_runtime_log(
        &fixture("runtime-stalled.jsonl"),
        &IncidentConfig::default(),
    )
    .unwrap();
    assert_eq!(scan.len(), 1);
    let want = scan.first().unwrap().clone();
    let d = tempfile::tempdir().unwrap();
    let stored = ingest(d.path(), &scan).unwrap();
    let id = &stored.first().unwrap().id;
    let back = show(d.path(), id).unwrap();
    assert_eq!(back.fingerprint, want.fingerprint);
    assert_eq!(back.count, want.count);
    assert_eq!(&back.id, id);
}

#[test]
fn ingesting_twice_is_one_row_with_the_same_id() {
    let scan = scan_runtime_log(
        &fixture("runtime-stalled.jsonl"),
        &IncidentConfig::default(),
    )
    .unwrap();
    let d = tempfile::tempdir().unwrap();
    let a = ingest(d.path(), &scan).unwrap();
    let b = ingest(d.path(), &scan).unwrap();
    assert_eq!(a.first().unwrap().id, b.first().unwrap().id);
    assert_eq!(files_in(d.path()), 1);
}

#[test]
fn two_fingerprints_are_two_ids() {
    let t: String = (0..4_i64)
        .flat_map(|i| {
            ["run.start.refused", "worker.request.refused"].map(|kind| {
                format!(
                    "{{\"kind\":\"{kind}\",\"fields\":{{}},\"ts_ms\":{}}}\n",
                    (1_760_000_000 + i * 10) * 1000
                )
            })
        })
        .collect();
    let d = tempfile::tempdir().unwrap();
    let log = d.path().join("daemon.jsonl");
    std::fs::write(&log, t).unwrap();
    let scan = scan_runtime_log(&log, &IncidentConfig::default()).unwrap();
    assert_eq!(scan.len(), 2, "{scan:?}");
    let store = tempfile::tempdir().unwrap();
    let stored = ingest(store.path(), &scan).unwrap();
    assert_ne!(stored.first().unwrap().id, stored.get(1).unwrap().id);
    assert_eq!(files_in(store.path()), 2);
}

#[test]
fn the_clean_control_stores_nothing() {
    let scan =
        scan_runtime_log(&fixture("runtime-clean.jsonl"), &IncidentConfig::default()).unwrap();
    let d = tempfile::tempdir().unwrap();
    let stored = ingest(d.path(), &scan).unwrap();
    assert!(stored.is_empty());
    assert_eq!(files_in(d.path()), 0);
}

#[test]
fn an_unknown_id_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    assert!(show(d.path(), "0123456789abcdef").is_err());
    assert!(show(d.path(), "../etc/passwd").is_err());
}
