//! Automated resume / crash-recovery coverage (opus review F4).
//!
//! The single most valuable durability property in pass-1 §3.4: resume must
//! never re-invoke a completed node (`NodeFinished` carries the value), and a
//! node interrupted mid-flight (`NodeStarted` with no terminal event) is
//! re-run. This test drives `crash-resume.toml` (a → wait(6000ms) → c, both
//! `a` and `c` append to a log file), cancels the root context mid-`wait`,
//! then resumes and asserts the append-only log contains exactly one `a` and
//! one `c` — i.e. `a` was NOT re-executed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunEvent, Terminal};
use pii_core::{Budget, CapKind, CapSet, Ctx, Observer, PathScope, RecordingObserver, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_store::Store;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

fn ws_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn load_crash_resume() -> pii_graph::GraphSpec {
    let full = ws_root().join("examples").join("crash-resume.toml");
    let src =
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    load_toml(&src).unwrap_or_else(|e| panic!("load crash-resume.toml: {e}"))
}

/// A root context that grants `fs_write` under `_tmp` (so the `emit` nodes can
/// append to the log) and whose cancel token the test can fire from another
/// thread.
fn root_ctx(run: pii_abi::RunId) -> (Ctx, pii_core::CancelToken) {
    let caps = CapSet {
        kinds: CapKind::FS_READ | CapKind::FS_WRITE,
        fs_write: PathScope::new(vec![tmp_root()]),
        fs_read: PathScope::new(vec![tmp_root()]),
        ..CapSet::default()
    };
    let ctx = Ctx::with_observer(
        caps,
        Budget::unlimited(),
        Arc::new(RecordingObserver::new()) as Arc<dyn Observer>,
        TraceCtx { run, span: 0 },
    );
    let cancel = ctx.cancel_token().clone();
    (ctx, cancel)
}

#[test]
fn resume_never_reinvokes_completed_node() {
    let spec = load_crash_resume();
    let dir = tempfile::Builder::new()
        .prefix("resume-test-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).expect("store open"));
    let run_id = store.next_run_id().expect("run id");
    let log = dir.path().join("out.log");
    let input = serde_json::json!({ "log": log.to_string_lossy() });
    let reg = NodeRegistry::builtin();

    let (ctx, cancel) = root_ctx(run_id.clone());

    // Run in a thread; cancel the root mid-`wait` (b has a 6000 ms timer).
    let store2 = Arc::clone(&store);
    let reg2 = NodeRegistry::builtin();
    let run_id2 = run_id.clone();
    let handle = std::thread::spawn(move || run::run(&store2, &spec, &reg2, &ctx, &run_id2, input));

    // Cancel once `a` has appended its line -- a CONDITION, not a fixed sleep. Two facts measured here
    // (2026-10-10) decided the shape of this wait:
    //   * a fixed 300 ms sleep was a latent flake: under load it elapsed before `a` had run at all, and
    //     the assertion below then failed for a reason that had nothing to do with resume (it failed
    //     once inside fresh-gate);
    //   * polling the JOURNAL for "b started" is worse: the journal is write-behind, and a flush carried
    //     `NodeStarted(b)` and `NodeFinished(b)` of the same tick TOGETHER at 6.1 s -- so that wait only
    //     returns after `b` has already finished, cancelling too late (the run completes, `b` is never
    //     mid-flight, and the NodeInterrupted assertion fails).
    // `a`'s log write is immediate, so it is the honest "a is done, b is next" signal.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if std::fs::read_to_string(&log).is_ok_and(|s| s.contains('a')) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "within 15s: `a` must append its line; log: {:?}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    cancel.cancel();
    let first = handle.join().expect("run thread");
    // The run is torn down mid-`wait`; it returns Cancelled (or the wait node
    // returns Cancelled and the run fails). Either way the run is not finished.
    let _ = first;

    // The log must contain exactly one `a` (a completed before the cancel).
    let log_a = std::fs::read_to_string(&log).expect("read log");
    assert_eq!(
        log_a.matches('a').count(),
        1,
        "a must be appended exactly once: {log_a:?}"
    );

    // Resume: replay the journal, continue the loop. `a` must NOT re-run.
    let (ctx2, _) = root_ctx(run_id.clone());
    let terminal = run::resume(&store, &reg, &ctx2, &run_id, None).expect("resume");
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "resume must finish Success, got {terminal:?}"
    );

    // The log now has exactly one `a` and one `c`.
    let log_final = std::fs::read_to_string(&log).expect("read log final");
    assert_eq!(
        log_final.matches('a').count(),
        1,
        "a must NOT be re-run: {log_final:?}"
    );
    assert_eq!(
        log_final.matches('c').count(),
        1,
        "c must run exactly once: {log_final:?}"
    );

    // The journal has exactly one NodeFinished for `a`.
    let journal = store.read_journal(&run_id).expect("journal");
    let a_finished = journal
        .iter()
        .filter(|(_, ev)| matches!(ev, RunEvent::NodeFinished { node, .. } if node.as_str() == "a"))
        .count();
    assert_eq!(
        a_finished, 1,
        "a must have exactly one NodeFinished in the journal"
    );
    // Spec 11 J3: `b` was cancelled mid-wait (NodeStarted, no terminal event),
    // so resume must have appended exactly one synthetic NodeInterrupted for
    // it, ordered before the Resumed event.
    let b_interrupted = journal
        .iter()
        .filter(|(_, ev)| matches!(ev, RunEvent::NodeInterrupted { node } if node.as_str() == "b"))
        .count();
    assert_eq!(
        b_interrupted, 1,
        "b must have exactly one NodeInterrupted in the journal (it was mid-flight when cancelled)"
    );
    let b_seq = journal
        .iter()
        .find_map(|(seq, ev)| {
            matches!(ev, RunEvent::NodeInterrupted { node } if node.as_str() == "b").then_some(*seq)
        })
        .expect("b NodeInterrupted seq");
    let resumed_seq = journal
        .iter()
        .find_map(|(seq, ev)| matches!(ev, RunEvent::Resumed { .. }).then_some(*seq))
        .expect("Resumed seq");
    assert!(
        b_seq < resumed_seq,
        "NodeInterrupted must precede Resumed (got {b_seq} >= {resumed_seq})"
    );
    // No NodeInterrupted for `a`: it completed, so it was not interrupted.
    assert!(
        !journal.iter().any(
            |(_, ev)| matches!(ev, RunEvent::NodeInterrupted { node } if node.as_str() == "a")
        ),
        "a completed before the cancel and must NOT be marked interrupted"
    );
}

/// Spec 30 T1 (acceptance): a suspended `timer` node whose wheel entry has
/// fired resumes to `Outcome::Next { port: "fired" }` instead of
/// re-suspending. The journal must contain exactly one `NodeFinished` for the
/// timer node with port `fired` — NOT a second `Suspended`.
#[test]
fn timer_resumes_to_fired_port() {
    let spec = load_toml(
        r#"
name = "timer-fire"
entry = "tick"
caps = []

[[node]]
id = "tick"
kind = "timer"
in_ms = 1

[[node]]
id = "done"
kind = "transform"
value = "done"

[[edge]]
from = "tick"
port = "fired"
to = "done"
kind = "data"
"#,
    )
    .expect("load timer-fire graph");

    let dir = tempfile::Builder::new()
        .prefix("timer-fire-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).expect("store open"));
    let run_id = store.next_run_id().expect("run id");
    let reg = NodeRegistry::builtin();

    let (ctx, _) = root_ctx(run_id.clone());
    let terminal =
        run::run(&store, &spec, &reg, &ctx, &run_id, serde_json::json!({})).expect("run");
    // The timer suspends immediately (in_ms=1 is in the future at dispatch).
    assert!(
        matches!(terminal, Terminal::Suspended { .. }),
        "first dispatch must suspend, got {terminal:?}"
    );

    // Wait past the fire time, then resume with the fired key.
    std::thread::sleep(Duration::from_millis(20));
    let (ctx2, _) = root_ctx(run_id.clone());
    let terminal2 = run::resume(&store, &reg, &ctx2, &run_id, None).expect("resume");
    assert!(
        matches!(terminal2, Terminal::Success { .. }),
        "resume must finish Success, got {terminal2:?}"
    );

    // The journal has exactly one NodeFinished for tick with port "fired".
    let journal = store.read_journal(&run_id).expect("journal");
    let tick_finished = journal
        .iter()
        .filter(|(_, ev)| {
            matches!(ev, RunEvent::NodeFinished { node, port, .. } if node.as_str() == "tick" && port.as_str() == "fired")
        })
        .count();
    assert_eq!(
        tick_finished, 1,
        "tick must have exactly one NodeFinished{{port:fired}}: {journal:?}"
    );
    // And NO second Suspended for tick.
    let tick_suspended = journal
        .iter()
        .filter(|(_, ev)| matches!(ev, RunEvent::Suspended { node, .. } if node.as_str() == "tick"))
        .count();
    assert_eq!(
        tick_suspended, 1,
        "tick must suspend exactly once (the first dispatch), not re-suspend: {journal:?}"
    );
}

/// Spec 30 T3: replaying a run whose timer already FINISHED does NOT
/// re-dispatch the timer (once-per-fire). Run to completion, then resume
/// again — the timer's dispatch count must stay 2 (first dispatch + the
/// fire dispatch), not 3.
#[test]
fn timer_does_not_refire_on_replay() {
    let spec = load_toml(
        r#"
name = "timer-once"
entry = "tick"
caps = []

[[node]]
id = "tick"
kind = "timer"
in_ms = 1

[[node]]
id = "done"
kind = "transform"
value = "done"

[[edge]]
from = "tick"
port = "fired"
to = "done"
kind = "data"
"#,
    )
    .expect("load timer-once graph");

    let dir = tempfile::Builder::new()
        .prefix("timer-once-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).expect("store open"));
    let run_id = store.next_run_id().expect("run id");
    let reg = NodeRegistry::builtin();

    // Run, wait past the fire time, resume to completion.
    let (ctx, _) = root_ctx(run_id.clone());
    let _ = run::run(&store, &spec, &reg, &ctx, &run_id, serde_json::json!({})).expect("run");
    std::thread::sleep(Duration::from_millis(20));
    let (ctx2, _) = root_ctx(run_id.clone());
    let terminal = run::resume(&store, &reg, &ctx2, &run_id, None).expect("resume");
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "resume must finish Success, got {terminal:?}"
    );

    // Resume AGAIN after completion — the timer must NOT re-dispatch.
    let (ctx3, _) = root_ctx(run_id.clone());
    let _ = run::resume(&store, &reg, &ctx3, &run_id, None).expect("resume again");

    // Count NodeStarted for tick — must be exactly 2 (first dispatch + the
    // fire dispatch), NOT 3 (no refire on replay of a completed timer).
    let journal = store.read_journal(&run_id).expect("journal");
    let tick_started = journal
        .iter()
        .filter(
            |(_, ev)| matches!(ev, RunEvent::NodeStarted { node, .. } if node.as_str() == "tick"),
        )
        .count();
    assert_eq!(
        tick_started, 2,
        "tick must be dispatched exactly twice (first + fire), no refire on replay: {journal:?}"
    );
}
