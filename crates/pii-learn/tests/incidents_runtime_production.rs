//! Spec 160: the runtime reader against COPIES of the daemon's real logs.
//!
//! Fixtures (tests/fixtures/production/), copied 2026-10-16, never hand-written:
//! - `pii.log`, `audit.log`: `cp /projects/_tmp/rev063-6pbi/{pii,audit}.log` — the
//!   production daemon's own runtime and audit logs, as copied by the 063 review
//!   on 2026-10-15 (one start, five audit start markers).
//! - `rot/pii.log`, `rot/pii.log.1`: `cp /projects/_tmp/v104rot-TBqo/logs/pii.log{,.1}`
//!   — a real daemon's rotated pair (`--log-rotate-bytes`); the start is only in `.1`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::IncidentConfig;
use pii_learn::incident_runtime::scan_runtime;
use std::path::{Path, PathBuf};

fn fx(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/production")
        .join(name)
}

fn grep_count(p: &Path, needle: &str) -> usize {
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| l.contains(needle))
        .count()
}

#[test]
fn every_start_in_the_real_log_is_reported() {
    let p = fx("pii.log");
    let s = scan_runtime(&p, &IncidentConfig::default()).unwrap();
    let n = grep_count(&p, "\"kind\":\"daemon.started\"");
    assert!(n > 0);
    assert_eq!(s.starts.len(), n);
    assert!(s.incidents.is_empty(), "{:?}", s.incidents);
    let t = s.starts.first().unwrap().time_to_notice_ms().unwrap();
    assert!(t > 0, "{t}");
}

#[test]
fn the_real_audit_log_opens_nothing() {
    let s = scan_runtime(&fx("audit.log"), &IncidentConfig::default()).unwrap();
    assert!(grep_count(&fx("audit.log"), "daemon.sandbox.in_force") >= 5);
    assert!(s.incidents.is_empty(), "{:?}", s.incidents);
}

#[test]
fn an_event_only_in_the_rotated_file_is_seen() {
    let cur = fx("rot/pii.log");
    assert_eq!(grep_count(&cur, "daemon.started"), 0);
    let s = scan_runtime(&cur, &IncidentConfig::default()).unwrap();
    assert_eq!(s.starts.len(), 1);
    assert_eq!(s.files.len(), 2);
}

#[test]
fn a_log_with_no_events_opens_nothing() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("pii.log");
    std::fs::write(&p, "").unwrap();
    let s = scan_runtime(&p, &IncidentConfig::default()).unwrap();
    assert!(s.incidents.is_empty() && s.starts.is_empty());
}

#[test]
fn the_same_input_twice_is_identical() {
    for f in ["pii.log", "audit.log", "rot/pii.log"] {
        let c = IncidentConfig::default();
        assert_eq!(
            scan_runtime(&fx(f), &c).unwrap(),
            scan_runtime(&fx(f), &c).unwrap()
        );
    }
}
