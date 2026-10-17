//! Spec 173: the loop's lines are pii events, and one reader shows them beside the daemon's own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_learn::loop_events::{LoopEvent, append, merged, render};

fn ev(ts_ms: i64, kind: &str, detail: &str) -> LoopEvent {
    LoopEvent {
        ts_ms,
        cycle: "C1".into(),
        kind: kind.into(),
        detail: detail.into(),
    }
}

#[test]
fn loop_and_daemon_events_come_back_in_one_time_ordered_list() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("loop-events.ndjson");
    let log = dir.path().join("daemon.log");
    std::fs::write(
        &log,
        "{\"kind\":\"daemon.started\",\"ts_ms\":1500}\nnot json\n{\"kind\":\"run.finished\",\"ts_ms\":2500}\n",
    )
    .unwrap();
    append(&file, &ev(1000, "loop.cycle.start", "")).unwrap();
    append(&file, &ev(2000, "loop.dispatch", "session=abc")).unwrap();
    append(&file, &ev(3000, "loop.gate.verdict", "failed step: clippy")).unwrap();

    let lines = merged(&file, Some(&log), 10);
    let order: Vec<(&str, &str)> = lines
        .iter()
        .map(|l| (l.source.as_str(), l.kind.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("loop", "loop.cycle.start"),
            ("daemon", "daemon.started"),
            ("loop", "loop.dispatch"),
            ("daemon", "run.finished"),
            ("loop", "loop.gate.verdict"),
        ]
    );
    assert!(render(lines.get(2).unwrap()).contains("session=abc"));
}

#[test]
fn last_n_keeps_the_newest_and_a_missing_log_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("e.ndjson");
    for i in 0..5 {
        append(&file, &ev(i, "loop.x", &format!("n={i}"))).unwrap();
    }
    let lines = merged(&file, Some(&dir.path().join("absent.log")), 2);
    let details: Vec<&str> = lines.iter().map(|l| l.detail.as_str()).collect();
    assert_eq!(details, ["cycle=C1 n=3", "cycle=C1 n=4"]);
}
