//! Spec 322: a start is observable from its first moment. The daemon writes `daemon.starting` before
//! anything that can fail, so the runtime scan can tell "a start that never served" from "a scan that
//! raced a healthy boot".
//!
//! Measured before this spec (deployed log, 2026-10-31): 108 `daemon.started`, 0 start marks — the
//! `daemon-start-not-ready` rule could never fire.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::IncidentConfig;
use pii_learn::incident_runtime::{RuntimeIncident, scan_runtime_log};
use std::path::PathBuf;

/// One runtime event line as the daemon writes it, `t` seconds after a fixed epoch.
fn ev(kind: &str, t: i64) -> String {
    format!(
        "{{\"kind\":\"{kind}\",\"fields\":{{}},\"ts_ms\":{}}}\n",
        (1_760_000_000 + t) * 1000
    )
}

fn scan(text: &str) -> Vec<RuntimeIncident> {
    let d = tempfile::tempdir().unwrap();
    let p: PathBuf = d.path().join("daemon.jsonl");
    std::fs::write(&p, text).unwrap();
    scan_runtime_log(&p, &IncidentConfig::default()).unwrap()
}

fn count_not_ready(inc: &[RuntimeIncident]) -> usize {
    inc.iter()
        .filter(|i| i.fingerprint.contains("daemon-start-not-ready"))
        .count()
}

/// A daemon log is only a daemon log if it carries lifecycle events; without one the scan refuses to
/// speak about starts at all (`incident_runtime.rs:219`). Every fixture below therefore begins with a
/// healthy start/stop pair and then the start under test.
fn after_a_healthy_run(rest: &str) -> String {
    ev("daemon.started", 0) + &ev("daemon.stopped", 1) + rest
}

#[test]
fn one_boot_writes_two_marks_and_is_silent() {
    // The host's own sequence, measured 2026-11-02 17:03:48 and 17:03:58: `daemon.starting`, then the sandbox
    // mark, then `daemon.started` ten seconds later. Both marks belong to ONE boot; reading the second as a start
    // that never served is what made the probe report `daemon-start-not-ready latency_secs=0` on every restart.
    let inc = scan(&after_a_healthy_run(
        &(ev("daemon.starting", 10)
            + &ev("daemon.sandbox.in_force", 10)
            + &ev("daemon.started", 20)),
    ));
    assert_eq!(count_not_ready(&inc), 0, "{inc:?}");
}

#[test]
fn start_that_never_served_is_one_incident() {
    // two marks, no `daemon.started` between them: the first start died before serving
    let inc = scan(&after_a_healthy_run(
        &(ev("daemon.starting", 10) + &ev("daemon.starting", 12)),
    ));
    assert_eq!(count_not_ready(&inc), 1, "{inc:?}");
}

#[test]
fn a_healthy_start_is_silent() {
    // control: the pair mark -> started is a normal boot and must never be an incident
    let inc = scan(&after_a_healthy_run(
        &(ev("daemon.starting", 10) + &ev("daemon.started", 12)),
    ));
    assert_eq!(count_not_ready(&inc), 0, "{inc:?}");
}

#[test]
fn a_scan_racing_a_boot_does_not_cry() {
    // control: the log ends one second after the mark — the daemon is still booting
    let inc = scan(&after_a_healthy_run(&ev("daemon.starting", 10)));
    assert_eq!(count_not_ready(&inc), 0, "{inc:?}");
}

#[test]
fn a_start_lost_for_ten_minutes_is_reported() {
    // the mark is the last line and the log then runs for ten more minutes: that start never served
    let inc = scan(&after_a_healthy_run(
        &(ev("daemon.starting", 10) + &ev("probe.tick", 610)),
    ));
    assert_eq!(count_not_ready(&inc), 1, "{inc:?}");
}

#[test]
fn the_legacy_sandbox_mark_still_pairs() {
    // old logs carry `daemon.sandbox.in_force` as the start mark; their meaning must not change
    let inc = scan(&after_a_healthy_run(
        &(ev("daemon.sandbox.in_force", 10) + &ev("daemon.sandbox.in_force", 40)),
    ));
    assert_eq!(count_not_ready(&inc), 1, "{inc:?}");
}

#[test]
fn the_incident_names_the_mark_and_the_loss() {
    let inc = scan(&after_a_healthy_run(
        &(ev("daemon.starting", 10) + &ev("daemon.starting", 12)),
    ));
    let i = inc
        .iter()
        .find(|i| i.fingerprint.contains("daemon-start-not-ready"))
        .unwrap_or_else(|| panic!("no incident: {inc:?}"));
    assert!(i.first_seen < i.opened_at, "the window is closed: {i:?}");
}
