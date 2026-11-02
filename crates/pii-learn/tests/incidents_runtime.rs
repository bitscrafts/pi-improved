//! Spec 156: incidents from the daemon's own runtime log, read-only, no model.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::IncidentConfig;
use pii_learn::incident_runtime::scan_runtime_log;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn ev(kind: &str, t: i64, run: Option<&str>) -> String {
    let run = run.map_or(String::new(), |r| format!(",\"run\":\"{r}\""));
    format!(
        "{{\"kind\":\"{kind}\"{run},\"fields\":{{}},\"ts_ms\":{}}}\n",
        (1_760_000_000 + t) * 1000
    )
}

fn log_with(text: &str) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("daemon.jsonl");
    std::fs::write(&p, text).unwrap();
    (d, p)
}

#[test]
fn a_stalled_run_is_one_incident_naming_the_run() {
    let inc = scan_runtime_log(
        &fixture("runtime-stalled.jsonl"),
        &IncidentConfig::default(),
    )
    .unwrap();
    assert_eq!(inc.len(), 1, "{inc:?}");
    let i = inc.first().unwrap();
    assert!(i.fingerprint.contains("run-7f3a"), "{}", i.fingerprint);
    assert!(i.source.ends_with("runtime-stalled.jsonl"));
    assert!(i.opened_at > i.first_seen);
}

#[test]
fn a_cancelled_run_is_not_stalled() {
    // The host's own run, 2026-11-02: r000000004356 logged `run.cancelled` after five `cron.rearmed` lines and was
    // still reported as `run-stalled`, because only an outcome or a refused start counted as finished.
    let (_d, p) = log_with(
        &(ev("run.start", 0, Some("r4356"))
            + &ev("cron.rearmed", 100, Some("r4356"))
            + &ev("run.cancelled", 200, Some("r4356"))
            + &ev("probe.tick", 4000, None)),
    );
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert!(
        !inc.iter().any(|i| i.fingerprint.contains("run-stalled")),
        "{inc:?}"
    );
}

#[test]
fn control_a_run_that_never_ends_is_still_stalled() {
    // the same log without the cancelled line: this one really did stall and must still be reported
    let (_d, p) = log_with(
        &(ev("run.start", 0, Some("r4356"))
            + &ev("cron.rearmed", 100, Some("r4356"))
            + &ev("probe.tick", 4000, None)),
    );
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert!(
        inc.iter().any(|i| i.fingerprint.contains("run-stalled")),
        "{inc:?}"
    );
}

#[test]
fn the_clean_control_opens_nothing() {
    let inc =
        scan_runtime_log(&fixture("runtime-clean.jsonl"), &IncidentConfig::default()).unwrap();
    assert!(inc.is_empty(), "{inc:?}");
}

#[test]
fn five_identical_refusals_are_one_incident_with_count_five() {
    let mut t = ev("daemon.started", 0, None);
    for i in 0..5 {
        t.push_str(&ev("run.start.refused", 10 + i * 10, None));
    }
    let (_d, p) = log_with(&t);
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert_eq!(inc.len(), 1, "{inc:?}");
    assert_eq!(inc.first().unwrap().count, 5);
    // below the configured count: nothing.
    let (_d, p) = log_with(&(ev("run.start.refused", 1, None) + &ev("run.start.refused", 2, None)));
    assert_eq!(
        scan_runtime_log(&p, &IncidentConfig::default()).unwrap(),
        [] as [pii_learn::incident_runtime::RuntimeIncident; 0]
    );
}

#[test]
fn an_unexpected_restart_and_a_start_that_never_became_ready_open() {
    // started, then started again with no stop between: a crash.
    let t = ev("daemon.started", 0, None)
        + &ev("daemon.started", 60, None)
        + &ev("daemon.stopped", 90, None);
    let (_d, p) = log_with(&t);
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert_eq!(inc.len(), 1, "{inc:?}");
    assert!(inc.first().unwrap().fingerprint.contains("restart"));
    // a start marker followed by a second start, never ready.
    let t = ev("daemon.sandbox.in_force", 0, None)
        + &ev("daemon.sandbox.in_force", 30, None)
        + &ev("daemon.started", 31, None);
    let (_d, p) = log_with(&t);
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert_eq!(inc.len(), 1, "{inc:?}");
    assert!(inc.first().unwrap().fingerprint.contains("not-ready"));
}

#[test]
fn the_source_log_is_read_never_written() {
    let (d, p) = log_with(&std::fs::read_to_string(fixture("runtime-stalled.jsonl")).unwrap());
    let before = std::fs::read(&p).unwrap();
    let m0 = std::fs::metadata(&p).unwrap().modified().unwrap();
    let names = |d: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    let listing = names(d.path());
    assert_eq!(
        scan_runtime_log(&p, &IncidentConfig::default())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(std::fs::read(&p).unwrap(), before);
    assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), m0);
    assert_eq!(names(d.path()), listing);
}
