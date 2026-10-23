//! Spec 193: a cron occurrence is counted; a multiplied one is an incident.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::IncidentConfig;
use pii_learn::incident_runtime::{RuntimeIncident, scan_runtime_log};
use std::path::{Path, PathBuf};

fn line(kind: &str, t: i64, cron: &str) -> String {
    format!(
        "{{\"kind\":\"{kind}\",\"fields\":{{\"cron\":\"{cron}\"}},\"ts_ms\":{}}}\n",
        (1_760_000_000 + t) * 1000
    )
}

fn scan(text: &str) -> Vec<RuntimeIncident> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("daemon.jsonl");
    std::fs::write(&p, text).unwrap();
    occurrence(scan_runtime_log(&p, &IncidentConfig::default()).unwrap())
}

fn occurrence(all: Vec<RuntimeIncident>) -> Vec<RuntimeIncident> {
    all.into_iter()
        .filter(|i| i.fingerprint.starts_with("cron-occurrence-multiplied"))
        .collect()
}

#[test]
fn one_firing_is_silent() {
    assert_eq!(
        scan(&line("cron.fired", 0, "digest")),
        [] as [RuntimeIncident; 0]
    );
}

#[test]
fn a_rearm_is_not_a_firing() {
    let t = line("cron.fired", 0, "digest") + &line("cron.rearmed", 1, "digest");
    assert_eq!(scan(&t), [] as [RuntimeIncident; 0]);
}

#[test]
fn two_firings_a_day_apart_are_silent() {
    let t = line("cron.fired", 0, "digest") + &line("cron.fired", 86_400, "digest");
    assert_eq!(scan(&t), [] as [RuntimeIncident; 0]);
}

#[test]
fn two_firings_three_seconds_apart_are_one_incident() {
    let t = line("cron.fired", 0, "digest") + &line("cron.fired", 3, "digest");
    let inc = scan(&t);
    assert_eq!(inc.len(), 1, "{inc:?}");
    let i = inc.first().unwrap();
    assert_eq!(i.count, 2);
    assert_eq!(i.fingerprint, "cron-occurrence-multiplied:digest");
    assert_eq!(
        (i.first_seen, i.last_seen, i.opened_at),
        (1_760_000_000, 1_760_000_003, 1_760_000_003)
    );
}

#[test]
fn different_crons_do_not_cluster() {
    let t = line("cron.fired", 0, "a") + &line("cron.fired", 3, "b");
    assert_eq!(scan(&t), [] as [RuntimeIncident; 0]);
}

#[test]
fn a_log_from_before_the_event_opens_nothing() {
    let p: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/runtime-stalled.jsonl");
    let all = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert_eq!(occurrence(all), [] as [RuntimeIncident; 0]);
}
