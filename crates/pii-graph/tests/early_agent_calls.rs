//! Spec 289: a running node's agent-call observations (`coder.spawned`, `coder.exited`) reach the journal while the
//! node still runs, once each, in the order the node made them, and before its own `NodeFinished`. A fast node adds no
//! barrier transaction.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName, RunEvent};
use pii_core::{Budget, CapKind, CapSet, Ctx, NullObserver, ObsEvent, Observer, TraceCtx};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::run;
use pii_graph::toml_load::load_toml;
use pii_store::Store;

/// A stand-in coder: it reports a `coder.spawned` for pid 7, holds for `hold_ms`, then (when `exit`) reports the
/// matching `coder.exited` and holds for `tail_ms` more.
struct Observing;

struct ObservingNode {
    hold_ms: u64,
    exit: bool,
    tail_ms: u64,
}

impl NodeFactory for Observing {
    fn kind(&self) -> &'static str {
        "observing"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let field = |key: &str| {
            cfg.get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        Ok(Box::new(ObservingNode {
            hold_ms: field("hold_ms"),
            exit: cfg
                .get("exit")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            tail_ms: field("tail_ms"),
        }))
    }
}

impl GraphNode for ObservingNode {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let observe = |kind: &str, fields: serde_json::Value| {
            ctx.observer().event(&ObsEvent::NodeObserved {
                kind: kind.to_owned(),
                fields,
            });
        };
        observe(
            "coder.spawned",
            serde_json::json!({ "pid": 7, "argv0": "stand-in" }),
        );
        std::thread::sleep(Duration::from_millis(self.hold_ms));
        if self.exit {
            observe("coder.exited", serde_json::json!({ "pid": 7, "rc": 0 }));
            std::thread::sleep(Duration::from_millis(self.tail_ms));
        }
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!("done"),
        })
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(Observing));
    reg
}

/// A one-node graph whose `coder` node is the stand-in above.
fn graph(hold_ms: u64, exit: bool, tail_ms: u64) -> pii_graph::GraphSpec {
    load_toml(&format!(
        "name = \"calls\"\nentry = \"coder\"\ncaps = []\n[limits]\nmax_steps = 10\nmax_wall_ms = 30000\n\
         [[node]]\nid = \"coder\"\nkind = \"observing\"\nhold_ms = {hold_ms}\nexit = {exit}\ntail_ms = {tail_ms}\n"
    ))
    .unwrap()
}

fn ctx(run_id: &pii_abi::RunId) -> Ctx {
    Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::new(NullObserver) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    )
}

fn scratch() -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "early-calls").unwrap()
}

/// Whether the store's journal for `run_id` holds an observation of `kind`.
fn observed(store: &Store, run_id: &pii_abi::RunId, kind: &str) -> bool {
    store
        .read_journal(run_id)
        .unwrap()
        .iter()
        .any(|(_, e)| matches!(e, RunEvent::NodeObserved { kind: k, .. } if k == kind))
}

#[test]
fn a_spawn_is_in_the_live_store_while_the_node_still_runs() {
    let dir = scratch();
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).unwrap());
    let spec = graph(3500, false, 0);
    let run_id = store.next_run_id().unwrap();
    let (s2, r2) = (Arc::clone(&store), run_id.clone());
    let worker = std::thread::spawn(move || {
        run::run(
            &s2,
            &spec,
            &registry(),
            &ctx(&r2),
            &r2,
            serde_json::json!({}),
        )
        .unwrap();
    });
    // The node holds 3.5 s after its spawn; the window is 2 s. The spawn must reach the store before it returns.
    let end = Instant::now() + Duration::from_secs(3);
    let mut durable = false;
    while Instant::now() < end && !worker.is_finished() {
        if observed(&store, &run_id, "coder.spawned") {
            durable = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let still_running = !worker.is_finished();
    worker.join().unwrap();
    assert!(
        durable && still_running,
        "coder.spawned was not in the store while the node still ran (durable={durable}, running={still_running})"
    );
}

#[test]
fn a_late_exit_reaches_the_live_store_inside_the_window() {
    let dir = scratch();
    let store = Arc::new(Store::open(&dir.path().join("db.redb")).unwrap());
    // The exit comes 2.5 s after the spawn, then the node holds 4 s more: the exit must be durable inside that tail.
    let spec = graph(2500, true, 4000);
    let run_id = store.next_run_id().unwrap();
    let (s2, r2) = (Arc::clone(&store), run_id.clone());
    let worker = std::thread::spawn(move || {
        run::run(
            &s2,
            &spec,
            &registry(),
            &ctx(&r2),
            &r2,
            serde_json::json!({}),
        )
        .unwrap();
    });
    let end = Instant::now() + Duration::from_secs(6);
    let mut durable = false;
    while Instant::now() < end && !worker.is_finished() {
        if observed(&store, &run_id, "coder.exited") {
            durable = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let still_running = !worker.is_finished();
    worker.join().unwrap();
    assert!(
        durable && still_running,
        "coder.exited was not in the store while the node still ran (durable={durable}, running={still_running})"
    );
}

#[test]
fn each_call_is_journalled_once_and_before_the_node_finished() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let id = store.next_run_id().unwrap();
    run::run(
        &store,
        &graph(2500, true, 0),
        &registry(),
        &ctx(&id),
        &id,
        serde_json::json!({}),
    )
    .unwrap();
    let journal = store.read_journal(&id).unwrap();
    let position = |pred: &dyn Fn(&RunEvent) -> bool| {
        let found: Vec<usize> = journal
            .iter()
            .enumerate()
            .filter(|(_, (_, e))| pred(e))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(found.len(), 1, "expected exactly one match, got {found:?}");
        found[0]
    };
    let spawned =
        position(&|e| matches!(e, RunEvent::NodeObserved { kind, .. } if kind == "coder.spawned"));
    let exited =
        position(&|e| matches!(e, RunEvent::NodeObserved { kind, .. } if kind == "coder.exited"));
    let finished = position(&|e| matches!(e, RunEvent::NodeFinished { .. }));
    assert!(
        spawned < exited && exited < finished,
        "journal order: spawned={spawned} exited={exited} finished={finished}"
    );
}

#[test]
fn a_fast_node_with_its_calls_costs_one_transaction_for_the_spawn() {
    let dir = scratch();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let reg = registry();
    let spec = graph(0, true, 0);
    // Warm the id block and the first-sight bodies, then measure a run.
    let warm = store.next_run_id().unwrap();
    run::run(
        &store,
        &spec,
        &reg,
        &ctx(&warm),
        &warm,
        serde_json::json!({}),
    )
    .unwrap();
    let id = store.next_run_id().unwrap();
    let before = store.write_txn_count();
    run::run(&store, &spec, &reg, &ctx(&id), &id, serde_json::json!({})).unwrap();
    let txns = store.write_txn_count() - before;
    // A fast run without calls costs at most 2 (see deadline_flush). Its spawn is made durable at once, so that run
    // with one spawn costs at most one more.
    assert!(
        txns <= 3,
        "a fast run with agent calls cost {txns} transactions; expected at most 3"
    );
}

/// A stand-in that records `count` numbered `coder.spawned` observations back to back, then holds for `hold_ms`.
struct Burst;

struct BurstNode {
    count: u64,
    hold_ms: u64,
}

impl NodeFactory for Burst {
    fn kind(&self) -> &'static str {
        "burst"
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let field = |key: &str| {
            cfg.get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        Ok(Box::new(BurstNode {
            count: field("count"),
            hold_ms: field("hold_ms"),
        }))
    }
}

impl GraphNode for BurstNode {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        for pid in 1..=self.count {
            ctx.observer().event(&ObsEvent::NodeObserved {
                kind: "coder.spawned".to_owned(),
                fields: serde_json::json!({ "pid": pid, "argv0": "stand-in" }),
            });
        }
        std::thread::sleep(Duration::from_millis(self.hold_ms));
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!("done"),
        })
    }
}

/// A burst reaches the journal once each and in the order it was recorded, whether the watcher takes it in one batch
/// or in several. (Every other test here records one observation, holds, then records the next, so the watcher never
/// holds a batch: a drain that reversed a batch passed them all.)
#[test]
fn a_burst_of_calls_is_journalled_once_each_in_order() {
    const COUNT: u64 = 2000;
    let mut reg = registry();
    reg.register(Arc::new(Burst));
    let spec = load_toml(&format!(
        "name = \"burst\"\nentry = \"coder\"\ncaps = []\n[limits]\nmax_steps = 10\nmax_wall_ms = 30000\n\
         [[node]]\nid = \"coder\"\nkind = \"burst\"\ncount = {COUNT}\nhold_ms = 2500\n"
    ))
    .unwrap();
    let dir = scratch();
    let store = Store::open(&dir.path().join("db.redb")).unwrap();
    let id = store.next_run_id().unwrap();
    run::run(&store, &spec, &reg, &ctx(&id), &id, serde_json::json!({})).unwrap();
    let pids: Vec<u64> = store
        .read_journal(&id)
        .unwrap()
        .into_iter()
        .filter_map(|(_, e)| match e {
            RunEvent::NodeObserved { kind, fields, .. } if kind == "coder.spawned" => {
                fields["pid"].as_u64()
            }
            _ => None,
        })
        .collect();
    let want: Vec<u64> = (1..=COUNT).collect();
    assert_eq!(
        pids.len(),
        want.len(),
        "each observation is journalled once"
    );
    assert_eq!(pids, want, "in the order the node recorded them");
}
