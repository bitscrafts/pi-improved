//! Spec 150: an incident is opened from the system's own records, no model.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::{IncidentConfig, detect, read_rows, scan_incidents};
use std::fmt::Write as _;
use std::path::Path;

const FIXTURE: &str = include_str!("fixtures/cycle-last-detached.trimmed.log");

fn root_with(name: &str, text: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join(name), text).unwrap();
    d
}

/// `n` identical outcomes, one every `step` seconds from 21:00:00.
fn burst(outcome: &str, n: u32, step: u32) -> String {
    let mut s = String::from("[21:00:00Z] === cycle 20261011T210000Z\n");
    for i in 0..n {
        let t = i * step;
        let _ = writeln!(
            s,
            "[21:{:02}:{:02}Z] refusing\nOUTCOME={outcome}",
            t / 60 % 60,
            t % 60
        );
    }
    s
}

#[test]
fn the_preserved_log_yields_exactly_the_real_incidents() {
    let d = root_with("cycle-last-detached.log", FIXTURE);
    let inc = detect(&read_rows(d.path()), &IncidentConfig::default());
    let got: Vec<_> = inc
        .iter()
        .map(|i| {
            (
                i.fingerprint.as_str(),
                i.first_seen.as_str(),
                i.opened_at.as_str(),
                i.count,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (
                "dirty-tree",
                "2026-10-11T21:01:44Z",
                "2026-10-11T21:11:48Z",
                6
            ),
            (
                "verify-failed",
                "2026-10-13T14:44:59Z",
                "2026-10-13T14:49:30Z",
                3
            ),
        ]
    );
    // window-closed repeats three times in the fixture and `done` once: benign, never opened.
    assert!(
        inc.iter()
            .all(|i| i.fingerprint != "window-closed" && i.fingerprint != "done")
    );
}

#[test]
fn refusals_collapse_into_one_incident() {
    let d = root_with("cycle-20261011T210000Z.log", &burst("dirty-tree", 497, 5));
    let inc = detect(&read_rows(d.path()), &IncidentConfig::default());
    assert_eq!(inc.len(), 1);
    assert_eq!(inc.first().unwrap().count, 497);
}

#[test]
fn a_benign_fingerprint_never_opens() {
    let d = root_with("cycle-20261011T210000Z.log", &burst("window-closed", 10, 5));
    assert_eq!(
        detect(&read_rows(d.path()), &IncidentConfig::default()),
        [] as [pii_learn::incident::Incident; 0]
    );
    // and the list is config: take it away and the same rows open one.
    let cfg = IncidentConfig {
        benign: vec![],
        ..IncidentConfig::default()
    };
    assert_eq!(detect(&read_rows(d.path()), &cfg).len(), 1);
}

#[test]
fn below_the_threshold_opens_nothing() {
    let d = root_with("cycle-20261011T210000Z.log", &burst("dirty-tree", 2, 60));
    assert_eq!(
        detect(&read_rows(d.path()), &IncidentConfig::default()),
        [] as [pii_learn::incident::Incident; 0]
    );
    // three rows, but spread wider than the 30 minute window
    let d = root_with("cycle-20261011T210000Z.log", &burst("dirty-tree", 3, 1000));
    assert_eq!(
        detect(&read_rows(d.path()), &IncidentConfig::default()),
        [] as [pii_learn::incident::Incident; 0]
    );
}

#[test]
fn the_evidence_points_at_a_line() {
    let d = root_with("cycle-last-detached.log", FIXTURE);
    let rows = read_rows(d.path());
    assert_ne!(rows, [] as [pii_learn::incident::Row; 0]);
    for r in &rows {
        let text = std::fs::read_to_string(&r.path).unwrap();
        let line = text.lines().nth(r.line - 1).unwrap();
        assert!(
            line.contains(&r.fingerprint),
            "{} not in {line}",
            r.fingerprint
        );
    }
    for i in detect(&rows, &IncidentConfig::default()) {
        let (p, l) = i.evidence_parts().unwrap();
        assert!(Path::new(p).exists());
        let text = std::fs::read_to_string(p).unwrap();
        assert!(text.lines().nth(l - 1).unwrap().contains(&i.fingerprint));
    }
}

#[test]
fn a_rescan_reports_nothing_new_and_a_longer_log_raises_the_count() {
    let d = root_with("cycle-20261011T210000Z.log", &burst("dirty-tree", 5, 60));
    let cfg = IncidentConfig::default();
    let first = scan_incidents(d.path(), &cfg).unwrap();
    assert_eq!((first.len(), first.first().unwrap().1), (1, true));
    let again = scan_incidents(d.path(), &cfg).unwrap();
    assert!(again.iter().all(|(_, new)| !new));
    std::fs::write(
        d.path().join("cycle-20261011T210000Z.log"),
        burst("dirty-tree", 9, 60),
    )
    .unwrap();
    let more = scan_incidents(d.path(), &cfg).unwrap();
    assert!(more.iter().all(|(_, new)| !new));
    let file = std::fs::read_to_string(d.path().join("incidents.jsonl")).unwrap();
    assert_eq!(file.lines().count(), 1);
    assert!(file.contains("\"count\":9"));
}

#[test]
fn the_gate_and_the_daemon_are_sources_too() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("export")).unwrap();
    std::fs::write(
        d.path().join("export/gate-output.log"),
        "test a::b ... ok\ntest quarantine::refused ... FAILED\n",
    )
    .unwrap();
    let ev = |k: &str, ms: u64| format!("{{\"kind\":\"{k}\",\"ts_ms\":{ms}}}\n");
    let mut daemon = String::new();
    for i in 0..3 {
        daemon.push_str(&ev("run.start.refused", 1_000_000 + i * 1000));
    }
    daemon.push_str(&ev("graph.run.started", 1_000_000));
    std::fs::write(d.path().join("daemon.log"), daemon).unwrap();
    let rows = read_rows(d.path());
    assert!(
        rows.iter()
            .any(|r| r.source == "gate" && r.fingerprint == "quarantine::refused")
    );
    let inc = detect(&rows, &IncidentConfig::default());
    assert_eq!(inc.len(), 1);
    assert_eq!(inc.first().unwrap().fingerprint, "run.start.refused");
}
