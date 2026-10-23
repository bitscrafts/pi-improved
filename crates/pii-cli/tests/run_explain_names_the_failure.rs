//! `pii run explain` must say WHY an agent exited non-zero. Two real failures on record read as bare numbers:
//! the haiku probe that died on root (`exited 1`, the cause only in the raw journal) and a pi run on a bad model
//! (`exited 1; criteria passed 1/1`, the 404 nowhere). The fields below are copied from those runs' `coder.exited`
//! events in the local daemon log, not typed by hand. Controls: a clean exit gains no failure text, and a failed
//! exit with an empty stderr still says it failed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus, Terminal};
use pii_store::Store;

fn observed(kind: &str, fields: serde_json::Value) -> RunEvent {
    RunEvent::NodeObserved {
        node: NodeId::from("code"),
        kind: kind.to_owned(),
        fields,
    }
}

fn exited(rc: i64, criteria_run: i64, criteria_passed: i64, stderr: &str) -> serde_json::Value {
    serde_json::json!({
        "pid": 4242, "rc": rc, "dur_ms": 408, "stop_reason": "end_turn",
        "criteria_run": criteria_run, "criteria_passed": criteria_passed,
        "criteria_failed": criteria_run - criteria_passed, "first_failure": "",
        "agent_stderr_bytes": stderr.len().to_string(), "agent_stderr_head": stderr,
    })
}

fn explain(run: &str, fields: serde_json::Value) -> String {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "explain-cli-").unwrap();
    let db = dir.path().join("store.redb");
    let store = Store::open(&db).unwrap();
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: "explain-test".to_owned(),
        status: RunStatus::Running,
        seq: 0,
        dur_ms: 0,
        revision: String::new(),
        process_start_ms: 1_000,
        created_ms: 1_000,
    };
    let events = [
        RunEvent::NodeStarted {
            node: NodeId::from("code"),
            at_ms: 10,
        },
        observed(
            "coder.spawned",
            serde_json::json!({"pid": 4242, "agent": "claude", "model": "haiku", "image_digest": "", "argv_digest": "00"}),
        ),
        observed("coder.exited", fields),
        RunEvent::NodeFinished {
            node: NodeId::from("code"),
            port: pii_abi::PortName::from("err"),
            value: serde_json::json!({"reason": "worker exited"}),
            value_hash: String::new(),
            at_ms: 10,
            dur_ms: 408,
        },
    ];
    for e in &events {
        store.append_event(&mut header, e).unwrap();
    }
    header.status = RunStatus::Failed;
    store
        .append_event(
            &mut header,
            &RunEvent::Finished {
                terminal: Terminal::Failure {
                    error: "worker exited".to_owned(),
                },
                output_hash: String::new(),
            },
        )
        .unwrap();
    // redb admits one opener: the binary below opens the same file, so this handle must be gone first.
    drop(store);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "explain", run, "--db"])
        .arg(&db)
        .env("PII_RUNTIME_DIR", dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_root_refusal_is_named_in_the_explanation() {
    let text = explain(
        "run-root",
        exited(
            1,
            0,
            0,
            "--dangerously-skip-permissions cannot be used with root/sudo privileges for security reasons\n",
        ),
    );
    assert!(
        text.contains("root/sudo privileges"),
        "the cause is shown: {text}"
    );
}

#[test]
fn a_bad_model_404_is_named_even_though_the_criterion_passed() {
    let text = explain(
        "run-404",
        exited(
            1,
            1,
            1,
            "Error: Provider error: ollama-cloud: OpenAI API error (HTTP 404): model not found",
        ),
    );
    assert!(text.contains("HTTP 404"), "the cause is shown: {text}");
    assert!(
        text.contains("agent failed"),
        "a passed criterion must not read as the verdict of a failed agent: {text}"
    );
}

#[test]
fn a_failed_exit_with_no_stderr_still_says_it_failed() {
    let text = explain("run-quiet", exited(3, 1, 1, ""));
    assert!(text.contains("agent failed"), "{text}");
}

#[test]
fn a_clean_exit_gains_no_failure_text() {
    let text = explain("run-clean", exited(0, 1, 1, ""));
    assert!(!text.contains("agent failed"), "near-miss: {text}");
}
