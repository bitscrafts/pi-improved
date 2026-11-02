//! Spec 231 R2/R4: a refused node's trace line says what the image probe found. A digest when a probe answered, `missing`
//! when the runtime said the image is absent, and `probe-failed` when the runtime could not answer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use pii_abi::{NodeId, RunEvent, RunHeader, RunId, RunStatus};
use pii_store::Store;

const DIGEST: &str = "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

/// Records one refused run: node `code` journals `worker.start.refused` with `image_digest` and `reason`, then fails.
fn record_refusal(store: &Store, run: &str, image_digest: &str, reason: &str) {
    let mut header = RunHeader {
        run: RunId::from(run),
        graph: "image-status".to_owned(),
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
        RunEvent::NodeObserved {
            node: NodeId::from("code"),
            kind: "worker.start.refused".to_owned(),
            fields: serde_json::json!({
                "agent": "pi",
                "model": "claude-test",
                "reason": reason,
                "image_digest": image_digest,
            }),
        },
        RunEvent::NodeFailed {
            node: NodeId::from("code"),
            error: reason.to_owned(),
            retryable: false,
            attempt: 1,
        },
    ];
    for event in &events {
        store.append_event(&mut header, event).unwrap();
    }
}

/// The trace line of node `code` in run `run`, read through the built `pii` binary with no daemon.
fn code_line(dir: &std::path::Path, db: &std::path::Path, run: &str) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .args(["run", "trace", run, "--db"])
        .arg(db)
        .env("PII_RUNTIME_DIR", dir)
        // State (and its durable stores) must be this test's own: the operator's are
        // held by any daemon already running on the machine.
        .env("PII_STATE_DIR", dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "trace exits zero: {:?} {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    stdout
        .lines()
        .find(|l| l.contains(" code "))
        .unwrap_or_else(|| panic!("a line for node code: {stdout}"))
        .to_owned()
}

#[test]
fn image_slot_names_the_probe_outcome() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "image-status-").unwrap();
    let db = dir.path().join("store.redb");
    {
        let store = Store::open(&db).unwrap();
        // A probe answered with a digest, and the refusal came after it.
        record_refusal(
            &store,
            "run-probed",
            DIGEST,
            "container refused after the probe",
        );
        // The runtime could not answer the probe: its lock refused it.
        record_refusal(
            &store,
            "run-probe-failed",
            "probe-failed",
            "Error: failed to open 2048 locks in /libpod_rootless_lock_1000: permission denied",
        );
        // The control: the runtime answered that the image is absent.
        record_refusal(&store, "run-missing", "missing", "image is absent");
    }

    let probed = code_line(dir.path(), &db, "run-probed");
    assert!(
        probed.contains("image=abcdef01 "),
        "a digest slot: `{probed}`"
    );

    let failed = code_line(dir.path(), &db, "run-probe-failed");
    assert!(
        failed.contains("image=probe-failed "),
        "the probe-failed slot: `{failed}`"
    );
    assert!(
        !failed.contains("image=missing"),
        "a runtime that did not answer is not missing: `{failed}`"
    );

    let missing = code_line(dir.path(), &db, "run-missing");
    assert!(
        missing.contains("image=missing "),
        "an absent image still prints missing: `{missing}`"
    );
}
