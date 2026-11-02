//! Spec 343 R3: a failure inside a called graph names BOTH the call and the node.
//!
//! The called graph's nodes are namespaced (`sub__boom`), so the failing id alone already says which call it was
//! in — but only if the run reports it. A run that swallows the inner id and reports the host's exit instead would
//! leave an operator unable to tell which of two calls failed. This file proves the id survives to the terminal and
//! the journal, and the control proves the same machinery names a host-side node without a prefix.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_store::Store;

/// A called graph whose single node refuses — a deterministic run-time failure with no external dependency.
const SUB_FAILS: &str = r#"
name = "sub"
exit = "boom"

[[node]]
id = "boom"
kind = "refuse"
reason = "the criteria table has no row for this item"
"#;

const HOST_CALLS: &str = r#"
name = "host"
entry = "sub"
exit = "sub"

[[node]]
id = "sub"
kind = "call"
graph = "sub"
sha256 = "PIN"
caps = []
"#;

/// The control: the same refusing node, in the host itself, with no call around it.
const HOST_REFUSES: &str = r#"
name = "host"
entry = "boom"
exit = "boom"

[[node]]
id = "boom"
kind = "refuse"
reason = "the criteria table has no row for this item"
"#;

fn tmp_root() -> PathBuf {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("create crate-local _tmp");
    root
}

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
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

/// Loads `host` with `subs`, runs it, and returns the terminal, the journal, and the call records.
fn run_host(
    host: &str,
    subs: &[(&str, &str)],
) -> (Terminal, Vec<(u64, RunEvent)>, Vec<fragment::CallRecord>) {
    let root = tempfile::Builder::new()
        .prefix("call-failure-")
        .tempdir_in(tmp_root())
        .expect("tempdir under _tmp");
    let graphs = root.path().join("graphs");
    std::fs::create_dir_all(&graphs).expect("mkdir graphs");
    for (name, src) in subs {
        std::fs::write(graphs.join(format!("{name}.toml")), src).expect("write graph");
    }
    let woven: Woven = fragment::load_host_with_graphs(
        host,
        &root.path().join("fragments"),
        std::slice::from_ref(&graphs),
        &NodeRegistry::builtin(),
    )
    .expect("host loads");
    let store_dir = tempfile::Builder::new()
        .prefix("store-")
        .tempdir_in(tmp_root())
        .expect("tempdir");
    let store = Store::open(&store_dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let reg = NodeRegistry::builtin();
    let terminal = run::run(
        &store,
        &woven.spec,
        &reg,
        &ctx(run_id.clone()),
        &run_id,
        serde_json::json!({}),
    )
    .expect("the run itself returns a terminal; a node failure is data, not an Err");
    let journal = store.read_journal(&run_id).expect("journal");
    (terminal, journal, woven.calls)
}

fn failed_nodes(journal: &[(u64, RunEvent)]) -> Vec<String> {
    journal
        .iter()
        .filter_map(|(_, ev)| match ev {
            RunEvent::NodeFailed { node, .. } => Some(node.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_failure_inside_a_call_names_the_call_and_the_node() {
    let sub = SUB_FAILS.to_owned();
    let host = HOST_CALLS.replace("PIN", &pin(&sub));
    let (terminal, journal, calls) = run_host(&host, &[("sub", &sub)]);
    assert!(
        !matches!(terminal, Terminal::Success { .. }),
        "a run whose only node refused must not succeed, got {terminal:?}"
    );
    let failed = failed_nodes(&journal);
    assert!(
        failed.iter().any(|n| n == "sub__boom"),
        "the failing node must be named `sub__boom` (the call id and the node), got {failed:?}"
    );
    // and the load recorded the call with the graph it inlined and the pin it verified
    let called = calls.iter().find(|c| c.call == "sub").unwrap_or_else(|| {
        panic!("no call record for `sub`: {calls:?}");
    });
    assert_eq!(called.graph, "sub");
    assert_eq!(
        called.sha256,
        pin(&sub),
        "the record must name the digest the load verified"
    );
}

#[test]
#[allow(non_snake_case)] // the name states what it controls for
fn CONTROL_a_host_failure_names_the_host_node() {
    let (terminal, journal, _calls) = run_host(HOST_REFUSES, &[]);
    assert!(
        !matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    let failed = failed_nodes(&journal);
    assert!(
        failed.iter().any(|n| n == "boom"),
        "a host-side failure keeps its own id, unprefixed: {failed:?}"
    );
    assert!(
        !failed.iter().any(|n| n.contains("__")),
        "no host node may be reported as if it were inside a call: {failed:?}"
    );
}
