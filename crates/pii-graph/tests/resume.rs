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

const TMP_ROOT: &str = "/projects/pi-improved/_tmp";

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
        fs_write: PathScope::new(vec![PathBuf::from(TMP_ROOT)]),
        fs_read: PathScope::new(vec![PathBuf::from(TMP_ROOT)]),
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
        .tempdir_in(TMP_ROOT)
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

    // Let `a` complete and `b` (wait) start, then cancel.
    std::thread::sleep(Duration::from_millis(300));
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
    let terminal = run::resume(&store, &reg, &ctx2, &run_id).expect("resume");
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
}
