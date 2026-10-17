//! Spec 173: `pii loop emit|events` — a cycle's events are queryable through the CLI, and a daemon
//! that is down neither stops the loop nor hides the gap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and the loop's report helper.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("loop-events-")
        .tempdir_in(root)
        .unwrap()
}

fn pii(args: &[&str], runtime: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(args)
        .env("PII_RUNTIME_DIR", runtime)
        .env("PII_CONNECT_TIMEOUT_MS", "300")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Runs `report <kind> <detail>` from the loop's helper, as one cycle does.
fn report(dir: &Path, cycle: &str, kind: &str, detail: &str) -> std::process::Output {
    let script = format!(
        ". '{}'; report {kind} '{detail}'",
        repo().join("scripts/autonomy/report.sh").display()
    );
    Command::new("bash")
        .args(["-c", &script])
        .env("PII_BIN", env!("CARGO_BIN_EXE_pii"))
        .env("PII_RUNTIME_DIR", dir.join("no-daemon"))
        .env("PII_CONNECT_TIMEOUT_MS", "300")
        .env("PII_LOOP_EVENTS", dir.join("loop-events.ndjson"))
        .env("PII_LOOP_STATE", dir)
        .env("CID", cycle)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn a_cycle_is_a_visible_sequence_through_the_cli() {
    let dir = scratch();
    let file = dir.path().join("e.ndjson");
    let f = file.to_str().unwrap();
    let rt = dir.path().join("rt");
    for (kind, detail) in [
        ("loop.cycle.start", "item=A38"),
        ("loop.dispatch", "session=s-1"),
        ("loop.gate.verdict", "failed step: clippy"),
    ] {
        pii(
            &[
                "loop", "emit", "--file", f, "--cycle", "C1", "--kind", kind, "--detail", detail,
            ],
            &rt,
        );
    }
    let out = pii(&["loop", "events", "--file", f, "--last", "10"], &rt);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let at = |k: &str| {
        text.find(k)
            .unwrap_or_else(|| panic!("{k} missing in {text}"))
    };
    assert!(at("loop.cycle.start") < at("loop.dispatch"));
    assert!(at("loop.dispatch") < at("loop.gate.verdict"));
    assert!(text.contains("failed step: clippy") && text.contains("session=s-1"));
}

#[test]
fn a_down_daemon_is_recorded_not_fatal_and_the_next_cycle_names_the_gap() {
    let dir = scratch();
    let first = report(dir.path(), "CYCLE-ONE", "loop.cycle.start", "item=A38");
    assert!(first.status.success(), "report must fail open: {first:?}");
    let events = std::fs::read_to_string(dir.path().join("loop-events.ndjson")).unwrap();
    assert!(
        events.contains("loop.cycle.start"),
        "the event is kept locally: {events}"
    );
    assert!(
        events.contains("daemon=down"),
        "the gap is recorded: {events}"
    );

    let second = report(dir.path(), "CYCLE-TWO", "loop.cycle.start", "item=A39");
    assert!(second.status.success());
    let said = String::from_utf8_lossy(&second.stdout);
    assert!(
        said.contains("CYCLE-ONE") && said.contains("not reported"),
        "next cycle's line must name the un-reported cycle: {said}"
    );
}
