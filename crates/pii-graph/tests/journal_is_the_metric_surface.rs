//! Spec 109 R3: the three graph metrics (`graph.node.dead`,
//! `graph.join.unsatisfiable`, `graph.batch.width`) are deliberately NOT daemon
//! log lines (per-tick events would flood it). The run journal records the same
//! facts, and `pii graph trace` reads them. This test pins that equivalence: what
//! the metric stream says equals what the journal says, so the documented query
//! is a faithful substitute.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_abi::{RunEvent, Terminal};
use pii_core::{Budget, CapSet, ContractObserver, Ctx, Observer, TraceCtx};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_observe::{Recorder, names};
use pii_store::Store;

struct Out {
    terminal: Terminal,
    journal: Vec<(u64, RunEvent)>,
    rec: Arc<Recorder>,
}

fn run_example(name: &str, input: serde_json::Value, validate: bool) -> Out {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
    std::fs::create_dir_all(&root).expect("crate-local _tmp");
    let ws = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let src = std::fs::read_to_string(ws.join("examples").join(name)).expect("read example");
    let spec = load_toml(&src).expect("load example");
    let dir = tempfile::Builder::new()
        .prefix("journal-surface-")
        .tempdir_in(root)
        .expect("tempdir under _tmp");
    let store = Store::open(&dir.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let rec = Arc::new(Recorder::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(ContractObserver::new(rec.clone())) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    let terminal = run::run_with_validate(
        &store,
        &spec,
        &NodeRegistry::builtin(),
        &ctx,
        &run_id,
        input,
        validate,
    )
    .expect("run");
    let journal = store.read_journal(&run_id).expect("journal");
    Out {
        terminal,
        journal,
        rec,
    }
}

fn metric_sum(rec: &Recorder, name: &str) -> u64 {
    rec.events_named(name)
        .iter()
        .filter_map(|e| e.fields.get("value").and_then(serde_json::Value::as_u64))
        .sum()
}

#[test]
fn dead_nodes_and_batch_widths_in_the_journal_equal_the_metric_stream() {
    let out = run_example(
        "routed-diamond.toml",
        serde_json::json!({"branch": "a"}),
        true,
    );
    let dead = out
        .journal
        .iter()
        .filter(|(_, e)| matches!(e, RunEvent::NodeDead { .. }))
        .count() as u64;
    assert!(dead >= 1, "the untaken branch must be journaled dead");
    assert_eq!(metric_sum(&out.rec, names::GRAPH_NODE_DEAD), dead);
    let widths: Vec<u64> = out
        .journal
        .iter()
        .filter_map(|(_, e)| match e {
            RunEvent::TickStarted { width, .. } => Some(u64::from(*width)),
            _ => None,
        })
        .collect();
    assert_ne!(widths, [] as [u64; 0]);
    let metric_widths: Vec<u64> = out
        .rec
        .events_named(names::GRAPH_BATCH_WIDTH)
        .iter()
        .filter_map(|e| e.fields.get("value").and_then(serde_json::Value::as_u64))
        .collect();
    assert_eq!(
        metric_widths, widths,
        "TickStarted.width is graph.batch.width"
    );
}

#[test]
fn an_unsatisfiable_join_is_in_the_terminal_the_trace_reads() {
    let out = run_example("unsatisfiable.toml", serde_json::json!({}), false);
    assert_eq!(metric_sum(&out.rec, names::GRAPH_JOIN_UNSATISFIABLE), 1);
    let Terminal::Failure { error } = &out.terminal else {
        panic!("want Failure, got {:?}", out.terminal);
    };
    assert!(error.contains("join can never be satisfied"), "{error}");
    assert!(
        out.journal.iter().any(|(_, e)| matches!(
            e,
            RunEvent::Finished {
                terminal: Terminal::Failure { .. },
                ..
            }
        )),
        "the terminal is journaled for `pii graph trace --terminal`"
    );
}
