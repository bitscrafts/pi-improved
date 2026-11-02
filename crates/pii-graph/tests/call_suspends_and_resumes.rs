//! Spec 343 R4/R6: a suspension inside a called graph is the HOST run's own suspension.
//!
//! Inclusion is load-time inline (spec 255): the called graph's nodes run in the host's run, numbered by the same
//! journal. So a `timer` inside a called graph should suspend the host run at the namespaced node (`sub__tick`),
//! and a resume should continue AT that node without re-entering the call from the top.
//!
//! Nothing tested this before: the only call tests that mention a gate use it as a pass-through, never as a
//! suspension. These tests run the graph and read the journal, because "the call was not re-entered" is a claim
//! about how many times a node started, and the journal is the only place that count is observable.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::fragment;
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_store::Store;

/// The called graph: a `before` node, then a timer that suspends, then `after`.
const SUB_TIMER: &str = r#"
name = "sub"
exit = "after"

[[node]]
id = "before"
kind = "transform"
value = "before"

[[node]]
id = "tick"
kind = "timer"
in_ms = 1

[[node]]
id = "after"
kind = "transform"
value = "done"

[[edge]]
from = "before"
port = "ok"
to = "tick"
kind = "data"

[[edge]]
from = "tick"
port = "fired"
to = "after"
kind = "data"
"#;

/// A host that calls it and reads the result by the call id.
const HOST: &str = r#"
name = "host"
entry = "sub"
exit = "read"

[[node]]
id = "sub"
kind = "call"
graph = "sub"
sha256 = "PIN"
caps = []

[[node]]
id = "read"
kind = "transform"
expr = "$.nodes.sub"
"#;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
}

struct Fixture {
    _root: tempfile::TempDir,
    graphs: PathBuf,
    fragments: PathBuf,
}

fn fixture(sub: &str) -> Fixture {
    let root = tempfile::Builder::new()
        .prefix("call-suspends-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let graphs = root.path().join("graphs");
    std::fs::create_dir_all(&graphs).expect("mkdir graphs");
    std::fs::write(graphs.join("sub.toml"), sub).expect("write sub");
    let fragments = root.path().join("fragments");
    Fixture {
        _root: root,
        graphs,
        fragments,
    }
}

fn ctx(run: pii_abi::RunId) -> Ctx {
    let obs = Arc::new(RecordingObserver::new());
    Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn Observer>,
        TraceCtx { run, span: 0 },
    )
}

/// Loads the host, dispatches it once (expecting a suspension), sleeps past the timer, resumes, and returns the
/// journal with the two terminals.
fn dispatch_sleep_resume(f: &Fixture, host: &str) -> (Terminal, Terminal, Vec<(u64, RunEvent)>) {
    let woven = fragment::load_host_with_graphs(
        host,
        &f.fragments,
        std::slice::from_ref(&f.graphs),
        &NodeRegistry::builtin(),
    )
    .expect("host loads");
    let dir = tempfile::Builder::new()
        .prefix("store-")
        .tempdir_in(tmp_root())
        .expect("tempdir");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let reg = NodeRegistry::builtin();

    let first = run::run(
        &store,
        &woven.spec,
        &reg,
        &ctx(run_id.clone()),
        &run_id,
        serde_json::json!({}),
    )
    .expect("first dispatch");
    std::thread::sleep(Duration::from_millis(25));
    let second = run::resume(&store, &reg, &ctx(run_id.clone()), &run_id, None).expect("resume");
    let journal = store.read_journal(&run_id).expect("journal");
    (first, second, journal)
}

fn starts(journal: &[(u64, RunEvent)], node: &str) -> usize {
    journal
        .iter()
        .filter(|(_, ev)| matches!(ev, RunEvent::NodeStarted { node: n, .. } if n.as_str() == node))
        .count()
}

#[test]
fn a_timer_inside_a_call_suspends_the_host_run() {
    let sub = SUB_TIMER.to_owned();
    let f = fixture(&sub);
    let host = HOST.replace("PIN", &pin(&sub));
    let (first, _, journal) = dispatch_sleep_resume(&f, &host);
    assert!(
        matches!(first, Terminal::Suspended { .. }),
        "the host run must suspend, got {first:?}"
    );
    // The terminal says WHEN the run should wake; the journal says WHICH node suspended it.
    let suspended = journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::Suspended { node, .. } => Some(node.as_str().to_owned()),
            _ => None,
        })
        .expect("a Suspended event in the journal");
    assert_eq!(
        suspended, "sub__tick",
        "the suspension must name the namespaced node inside the call"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_the_subgraph_alone_suspends_at_tick() {
    // the same graph, not called: it suspends at `tick` with no prefix, which is what the host run names `sub__tick`
    let sub = SUB_TIMER.to_owned();
    let f = fixture(&sub);
    let mut plain = sub.replace("exit = \"after\"", "entry = \"before\"\nexit = \"after\"");
    plain
        .push_str("\n[[edge]]\nfrom = \"before\"\nport = \"ok\"\nto = \"tick\"\nkind = \"data\"\n");
    let woven = fragment::load_host_with_graphs(
        &plain,
        &f.fragments,
        std::slice::from_ref(&f.graphs),
        &NodeRegistry::builtin(),
    )
    .expect("loads standalone");
    let dir = tempfile::Builder::new()
        .prefix("store-")
        .tempdir_in(tmp_root())
        .expect("tempdir");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let reg = NodeRegistry::builtin();
    let first = run::run(
        &store,
        &woven.spec,
        &reg,
        &ctx(run_id.clone()),
        &run_id,
        serde_json::json!({}),
    )
    .expect("dispatch");
    assert!(matches!(first, Terminal::Suspended { .. }), "got {first:?}");
    let journal = store.read_journal(&run_id).expect("journal");
    let suspended = journal
        .iter()
        .find_map(|(_, ev)| match ev {
            RunEvent::Suspended { node, .. } => Some(node.as_str().to_owned()),
            _ => None,
        })
        .expect("a Suspended event");
    assert_eq!(suspended, "tick", "uncalled, the graph suspends at `tick`");
}

#[test]
fn resuming_the_host_continues_inside_the_call_at_the_suspended_node() {
    let sub = SUB_TIMER.to_owned();
    let f = fixture(&sub);
    let host = HOST.replace("PIN", &pin(&sub));
    let (_, second, journal) = dispatch_sleep_resume(&f, &host);
    let Terminal::Success { output } = second else {
        panic!("resume must succeed, got {second:?}");
    };
    assert_eq!(output, serde_json::json!("done"), "the call's exit value");
    // A timer is INTERRUPTED at its suspension and re-dispatched on resume, so it starts twice (measured). That is
    // the timer's semantics, not the call's: the claim about the CALL is that it is not re-entered from its top,
    // which is `sub__before == 1` below (and the test that follows).
    assert_eq!(starts(&journal, "sub__after"), 1, "after ran exactly once");
}

#[test]
fn a_call_that_finished_before_the_suspend_is_not_rerun() {
    // `before` ran before the suspension; a resume that re-entered the call would start it a second time
    let sub = SUB_TIMER.to_owned();
    let f = fixture(&sub);
    let host = HOST.replace("PIN", &pin(&sub));
    let (_, _, journal) = dispatch_sleep_resume(&f, &host);
    assert_eq!(
        starts(&journal, "sub__before"),
        1,
        "resuming must not re-enter the call from its top"
    );
}

#[test]
fn an_edit_of_the_called_graph_after_dispatch_cannot_change_a_suspended_run() {
    // The expanded spec is stored content-addressed at run start, so a resume must continue on the copy it started
    // with. Re-reading `graphs/sub.toml` "for safety" would make a resume non-deterministic — and would let an edit
    // silently change the meaning of work already in flight.
    let sub = SUB_TIMER.to_owned();
    let f = fixture(&sub);
    let host = HOST.replace("PIN", &pin(&sub));
    let woven = fragment::load_host_with_graphs(
        &host,
        &f.fragments,
        std::slice::from_ref(&f.graphs),
        &NodeRegistry::builtin(),
    )
    .expect("host loads");
    let dir = tempfile::Builder::new()
        .prefix("store-")
        .tempdir_in(tmp_root())
        .expect("tempdir");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let reg = NodeRegistry::builtin();
    let first = run::run(
        &store,
        &woven.spec,
        &reg,
        &ctx(run_id.clone()),
        &run_id,
        serde_json::json!({}),
    )
    .expect("dispatch");
    assert!(matches!(first, Terminal::Suspended { .. }), "got {first:?}");

    // The operator edits the called graph: its exit now says something else entirely.
    let edited = SUB_TIMER.replace("value = \"done\"", "value = \"changed-after-dispatch\"");
    std::fs::write(f.graphs.join("sub.toml"), &edited).expect("edit the sub");

    std::thread::sleep(Duration::from_millis(25));
    let second = run::resume(&store, &reg, &ctx(run_id.clone()), &run_id, None).expect("resume");
    let Terminal::Success { output } = second else {
        panic!("resume must succeed, got {second:?}");
    };
    assert_eq!(
        output,
        serde_json::json!("done"),
        "the resume must run the copy the run started with, not the edited file"
    );
}
