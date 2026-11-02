//! Spec 346: a run with no terminal event is read from what the log says about it, not from the absence alone.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::incident::IncidentConfig;
use pii_learn::incident_runtime::{RuntimeIncident, scan_runtime_log};
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/production/runtime-0503.jsonl")
}

fn ev(kind: &str, t: i64, run: Option<&str>) -> String {
    let run = run.map_or(String::new(), |r| format!(",\"run\":\"{r}\""));
    format!(
        "{{\"kind\":\"{kind}\"{run},\"fields\":{{}},\"ts_ms\":{}}}\n",
        (1_760_000_000 + t) * 1000
    )
}

fn failed(run: &str, t: i64, error: &str) -> String {
    format!(
        "{{\"kind\":\"trajectory.write.failed\",\"run\":\"{run}\",\"fields\":{{\"error\":\"{error}\",\"run\":\"{run}\"}},\"ts_ms\":{}}}\n",
        (1_760_000_000 + t) * 1000
    )
}

fn log_with(text: &str) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("daemon.jsonl");
    std::fs::write(&p, text).unwrap();
    (d, p)
}

fn stalled(inc: &[RuntimeIncident]) -> usize {
    inc.iter()
        .filter(|i| i.fingerprint.starts_with("run-stalled"))
        .count()
}

#[test]
fn a_posted_run_is_not_stalled() {
    // The host's r4577: `run.start`, then the trajectory write failed, then the channel posted the result.
    // The run did its work; the missing terminal event is no evidence of a stall.
    let (_d, p) = log_with(
        &(ev("run.start", 0, Some("r4577"))
            + &failed("r4577", 100, "Permission denied (os error 13)")
            + &ev("channel.posted", 200, Some("r4577"))
            + &ev("probe.tick", 4000, None)),
    );
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert_eq!(stalled(&inc), 0, "{inc:?}");
}

#[test]
fn a_lost_trajectory_is_observability_lost() {
    let (_d, p) = log_with(
        &(ev("run.start", 0, Some("r4578"))
            + &failed("r4578", 100, "Permission denied (os error 13)")
            + &ev("probe.tick", 4000, None)),
    );
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    let lost = inc
        .iter()
        .find(|i| i.fingerprint == "observability-lost:r4578")
        .unwrap_or_else(|| panic!("{inc:?}"));
    assert!(
        lost.evidence.contains("Permission denied (os error 13)"),
        "{}",
        lost.evidence
    );
}

#[test]
fn control_a_run_that_never_ends_and_never_posts_is_still_stalled() {
    // the same shape with no channel post and no trajectory failure: nothing says it finished, so it still stalls
    let (_d, p) = log_with(
        &(ev("run.start", 0, Some("r4579"))
            + &ev("cron.rearmed", 100, Some("r4579"))
            + &ev("probe.tick", 4000, None)),
    );
    let inc = scan_runtime_log(&p, &IncidentConfig::default()).unwrap();
    assert_eq!(stalled(&inc), 1, "{inc:?}");
    assert!(
        inc.iter().any(|i| i.fingerprint == "run-stalled:r4579"),
        "{inc:?}"
    );
}

#[test]
fn the_0503_window_yields_its_verdicts() {
    let inc = scan_runtime_log(&fixture(), &IncidentConfig::default()).unwrap();
    assert_eq!(stalled(&inc), 0, "{inc:?}");
    let lost: Vec<&str> = inc
        .iter()
        .filter(|i| i.fingerprint.starts_with("observability-lost:"))
        .map(|i| i.fingerprint.as_str())
        .collect();
    assert_eq!(
        lost,
        [
            "observability-lost:r000000004577",
            "observability-lost:r000000004578"
        ],
        "{inc:?}"
    );
    let refusal = inc
        .iter()
        .find(|i| i.fingerprint == "refusal-repeated:channel.poll.failed")
        .unwrap_or_else(|| panic!("{inc:?}"));
    assert_eq!(refusal.count, 731, "{refusal:?}");
}
