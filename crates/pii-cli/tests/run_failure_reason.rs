//! Spec 293: a run that fails on an engine limit says which limit. `pii run get` carries a `reason` field and
//! `pii run explain` ends its sentence with the terminal's error; `pii run trace` shows a judge node's `crit=` and the
//! criteria that failed. The journals below are written directly, the way the engine journals its terminal event, so
//! the reason the CLI prints is the one the journal holds. Controls: a succeeded or cancelled run gains no reason.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, PortName, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

/// How the recorded run ended: its header status and, when it wrote one, its terminal event.
enum End {
    /// A `Finished` event carrying this terminal, and the status the engine sets with it.
    Terminal(Terminal, RunStatus),
    /// No terminal at all: the run's header stays `Running`, as after a cancelled teardown.
    None,
}

/// Records one run with a single node that ran and finished, then the given end. Returns the store's path.
fn record(
    dir: &std::path::Path,
    run: &str,
    node: &str,
    value: serde_json::Value,
    end: End,
) -> std::path::PathBuf {
    let db = dir.join("store.redb");
    let store = Store::open(&db).unwrap();
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: "limit-test".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 888_774,
        revision: String::new(),
        process_start_ms: 1_000,
        created_ms: 1_000,
    };
    let events = [
        RunEvent::NodeStarted {
            node: NodeId::from(node),
            at_ms: 10,
        },
        RunEvent::NodeFinished {
            node: NodeId::from(node),
            port: PortName::from("ok"),
            value,
            value_hash: String::new(),
            at_ms: 10,
            dur_ms: 3,
        },
    ];
    for e in &events {
        store.append_event(&mut header, e).unwrap();
    }
    if let End::Terminal(terminal, status) = end {
        header.status = status;
        store
            .append_event(
                &mut header,
                &RunEvent::Finished {
                    terminal,
                    output_hash: String::new(),
                },
            )
            .unwrap();
    }
    db
}

/// Runs `pii run <verb> <run> --db <db>` with no daemon reachable and returns its stdout.
fn pii(dir: &std::path::Path, db: &std::path::Path, verb: &[&str], run: &str) -> String {
    // Its own runtime dir: a daemon on this host must not answer for the store this test names (spec 220).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run"])
        .args(verb)
        .args([run, "--db"])
        .arg(db)
        .env("PII_RUNTIME_DIR", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pii run {verb:?} exits zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The failing run's `explain` sentence, named by the limit it stopped on.
fn failed_on(error: &str, run: &str) -> (String, String) {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "failure-reason-").unwrap();
    let db = record(
        dir.path(),
        run,
        "spec",
        serde_json::json!({}),
        End::Terminal(
            Terminal::Failure {
                error: error.to_owned(),
            },
            RunStatus::Failed,
        ),
    );
    let get = pii(dir.path(), &db, &["get"], run);
    let explain = pii(dir.path(), &db, &["explain"], run);
    (get, explain)
}

#[test]
fn a_wall_stop_names_the_wall() {
    let (get, explain) = failed_on("max_wall_ms exceeded", "run-wall");
    assert!(
        get.contains("\"reason\":\"max_wall_ms exceeded\""),
        "run get names the wall: {get}"
    );
    assert!(
        explain.contains("failed in 888774ms: max_wall_ms exceeded."),
        "explain ends with the wall: {explain}"
    );
}

#[test]
fn a_step_stop_names_the_steps() {
    let (get, explain) = failed_on("max_steps exceeded", "run-steps");
    assert!(
        get.contains("\"reason\":\"max_steps exceeded\""),
        "run get names the steps: {get}"
    );
    assert!(
        explain.contains("failed in 888774ms: max_steps exceeded."),
        "explain ends with the steps: {explain}"
    );
}

#[test]
fn a_budget_stop_names_the_budget() {
    let (get, explain) = failed_on("budget exhausted: Tokens", "run-budget");
    assert!(
        get.contains("\"reason\":\"budget exhausted: Tokens\""),
        "run get names the budget: {get}"
    );
    assert!(
        explain.contains("failed in 888774ms: budget exhausted: Tokens."),
        "explain ends with the budget: {explain}"
    );
}

#[test]
fn a_judge_row_counts_its_criteria_and_names_the_failing_one() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "failure-reason-").unwrap();
    let verdict = serde_json::json!({
        "verdict": "failed",
        "criteria": 5,
        "passed": 4,
        "results": [],
        "reason": "criterion failed: cargo test -q (rc 1)",
        "failing": ["cargo test -q"],
    });
    let db = record(
        dir.path(),
        "run-judge",
        "judge",
        verdict,
        End::Terminal(
            Terminal::Success {
                output: serde_json::json!({}),
            },
            RunStatus::Succeeded,
        ),
    );
    let trace = pii(dir.path(), &db, &["trace"], "run-judge");
    let row = trace
        .lines()
        .find(|l| l.contains(" judge "))
        .unwrap_or_else(|| panic!("a row for the judge node: {trace}"));
    assert!(row.contains("crit=4/5"), "the judge's count: {row}");
    assert!(
        row.contains("fail=cargo test -q"),
        "the failing criterion's text: {row}"
    );
}

#[test]
fn CONTROL_a_succeeded_run_has_no_reason() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "failure-reason-").unwrap();
    let db = record(
        dir.path(),
        "run-ok",
        "spec",
        serde_json::json!({}),
        End::Terminal(
            Terminal::Success {
                output: serde_json::json!({}),
            },
            RunStatus::Succeeded,
        ),
    );
    let get = pii(dir.path(), &db, &["get"], "run-ok");
    let explain = pii(dir.path(), &db, &["explain"], "run-ok");
    assert!(!get.contains("reason"), "no reason in run get: {get}");
    assert!(
        explain.starts_with("Run run-ok of limit-test succeeded in 888774ms."),
        "explain unchanged: {explain}"
    );
}

#[test]
fn CONTROL_a_cancelled_run_has_no_reason() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "failure-reason-").unwrap();
    let db = record(
        dir.path(),
        "run-cancelled",
        "spec",
        serde_json::json!({}),
        End::None,
    );
    let get = pii(dir.path(), &db, &["get"], "run-cancelled");
    let explain = pii(dir.path(), &db, &["explain"], "run-cancelled");
    assert!(!get.contains("reason"), "no reason in run get: {get}");
    assert!(
        !explain.contains("failed in"),
        "no failure sentence: {explain}"
    );
}

/// Two failing criteria are both on the judge row, in the order the judge listed them, separated by `; `. (Every other test here has one,
/// so a different separator, or only the first, passed them all.)
#[test]
fn two_failing_criteria_are_both_named_in_order() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "failure-reason-").unwrap();
    let verdict = serde_json::json!({
        "verdict": "failed",
        "criteria": 5,
        "passed": 3,
        "results": [],
        "reason": "criterion failed: first (rc 1)",
        "failing": ["first", "second"],
    });
    let db = record(
        dir.path(),
        "run-two",
        "judge",
        verdict,
        End::Terminal(
            Terminal::Success {
                output: serde_json::json!({}),
            },
            RunStatus::Succeeded,
        ),
    );
    let trace = pii(dir.path(), &db, &["trace"], "run-two");
    let row = trace
        .lines()
        .find(|l| l.contains(" judge "))
        .unwrap_or_else(|| panic!("a row for the judge node: {trace}"));
    assert!(row.contains("crit=3/5"), "the judge's count: {row}");
    assert!(
        row.contains("fail=first; second"),
        "both failing criteria, in order, joined by '; ': {row}"
    );
}

/// A run that failed and was then resumed to success holds TWO `Finished` events. The reason is the last one's, so a succeeded run has none.
/// (The first cut read the newest FAILURE, so it printed the old error on a run that finally succeeded.)
#[test]
fn a_later_success_hides_an_older_failure() {
    let dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "failure-reason-").unwrap();
    let db = record(
        dir.path(),
        "run-resumed",
        "spec",
        serde_json::json!({}),
        End::Terminal(
            Terminal::Failure {
                error: "max_wall_ms exceeded".to_owned(),
            },
            RunStatus::Failed,
        ),
    );
    // the resume: the same journal gets a second, successful terminal
    {
        let store = Store::open(&db).unwrap();
        let mut header = store.get_run(&RunId::from("run-resumed")).unwrap().unwrap();
        header.status = RunStatus::Succeeded;
        store
            .append_event(
                &mut header,
                &RunEvent::Finished {
                    terminal: Terminal::Success {
                        output: serde_json::json!({}),
                    },
                    output_hash: String::new(),
                },
            )
            .unwrap();
    }
    let get = pii(dir.path(), &db, &["get"], "run-resumed");
    let explain = pii(dir.path(), &db, &["explain"], "run-resumed");
    assert!(
        !get.contains("max_wall_ms"),
        "run get must not name the old failure: {get}"
    );
    assert!(
        !explain.contains("max_wall_ms"),
        "explain must not name the old failure: {explain}"
    );
}
