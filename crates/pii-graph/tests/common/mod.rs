//! Shared rig for the spec 186 tests: counting probe nodes (pure and effect
//! kinds) that can abort the run or fail while `broken` is set.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use pii_abi::{Effects, NodeId, NodeInput, Outcome, PiiError, PortName, RunId};
use pii_core::{Budget, CancelToken, CapKind, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::node::{GraphNode, NodeFactory, NodeRegistry};
use pii_graph::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};

/// Shared state the probe nodes report into.
#[derive(Clone, Default)]
pub struct Probe {
    pub counts: Arc<Mutex<BTreeMap<String, usize>>>,
    /// While true, a node with `"mode": "abort"` tears the run down and one
    /// with `"mode": "fail"` fails.
    pub broken: Arc<AtomicBool>,
    pub root_cancel: Arc<Mutex<Option<CancelToken>>>,
    /// Every execution's `(node, input.nodes)`, in order.
    pub seen: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl Probe {
    pub fn count(&self, node: &str) -> usize {
        self.counts.lock().get(node).copied().unwrap_or(0)
    }
}

struct ProbeNode {
    id: String,
    mode: String,
    probe: Probe,
}

impl GraphNode for ProbeNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        self.probe
            .seen
            .lock()
            .push((self.id.clone(), input.nodes.clone()));
        let n = {
            let mut c = self.probe.counts.lock();
            let e = c.entry(self.id.clone()).or_insert(0);
            *e += 1;
            let n = *e;
            drop(c);
            n
        };
        if self.probe.broken.load(Ordering::SeqCst) {
            match self.mode.as_str() {
                "abort" => {
                    if let Some(t) = self.probe.root_cancel.lock().as_ref() {
                        t.cancel();
                    }
                    return Err(PiiError::Cancelled);
                }
                "fail" => {
                    return Ok(Outcome::Fail {
                        error: "probe broken".into(),
                    });
                }
                _ => {}
            }
        }
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({"node": self.id, "n": n}),
        })
    }
}

struct ProbeFactory {
    kind: &'static str,
    effect: bool,
    probe: Probe,
}

impl NodeFactory for ProbeFactory {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn effects(&self) -> Effects {
        if self.effect {
            Effects::MUTATES_EXTERNAL
        } else {
            Effects::PURE
        }
    }
    fn is_effect(&self) -> bool {
        self.effect
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(ProbeNode {
            id: cfg["id"].as_str().unwrap_or("?").to_owned(),
            mode: cfg["mode"].as_str().unwrap_or("ok").to_owned(),
            probe: self.probe.clone(),
        }))
    }
}

/// Registry with `probe` (pure) and `effect` (journals its key) kinds.
pub fn registry(probe: &Probe) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(ProbeFactory {
        kind: "probe",
        effect: false,
        probe: probe.clone(),
    }));
    reg.register(Arc::new(ProbeFactory {
        kind: "effect",
        effect: true,
        probe: probe.clone(),
    }));
    reg
}

/// `a -> b -> c`; each entry is `(kind, mode)`.
pub fn chain(nodes: [(&str, &str); 3]) -> GraphSpec {
    let ids = ["a", "b", "c"];
    let spec_nodes: Vec<NodeSpec> = nodes
        .iter()
        .zip(ids)
        .map(|((kind, mode), id)| NodeSpec {
            id: NodeId::from(id),
            kind: (*kind).to_owned(),
            join: JoinPolicy::All,
            loop_budget: None,
            retry: RetryPolicy::default(),
            caps: 0,
            cfg: serde_json::json!({"id": id, "mode": mode}),
        })
        .collect();
    let edge = |from: &str, to: &str| Edge {
        from: NodeId::from(from),
        port: PortName::from("ok"),
        to: NodeId::from(to),
        kind: EdgeKind::Data,
    };
    GraphSpec {
        name: "chain".into(),
        caps: 0,
        nodes: spec_nodes,
        edges: vec![edge("a", "b"), edge("b", "c")],
        entry: NodeId::from("a"),
        limits: GraphLimits::default(),
    }
}

/// A fresh root context for `run`.
pub fn ctx(run: RunId) -> Ctx {
    let obs = Arc::new(RecordingObserver::new());
    Ctx::with_observer(
        CapSet::default(),
        Budget::unlimited(),
        obs as Arc<dyn Observer>,
        TraceCtx { run, span: 0 },
    )
}

/// A scratch directory under the crate's `_tmp`.
pub fn scratch(prefix: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), prefix).expect("scratch dir")
}

/// Writes a run whose journal is exactly `events` (after `Started`), as a
/// crashed daemon would have left it: the spec stored, header `Running`.
pub fn seed_run(store: &pii_store::Store, spec: &GraphSpec, events: &[pii_abi::RunEvent]) -> RunId {
    let graph = store.put_graph(&spec.to_json().unwrap()).unwrap();
    let input_hash = store.put_graph(b"null").unwrap();
    let id = store.next_run_id().unwrap();
    let mut header = pii_abi::RunHeader {
        run: id.clone(),
        graph,
        status: pii_abi::RunStatus::Running,
        seq: 0,
        dur_ms: 0,
    };
    let started = pii_abi::RunEvent::Started {
        graph: spec.name.clone(),
        input_hash,
    };
    store.append_event(&mut header, &started).unwrap();
    for ev in events {
        store.append_event(&mut header, ev).unwrap();
    }
    id
}

/// A `NodeFinished` for `node` carrying `value` on port `ok`.
pub fn finished(node: &str, value: serde_json::Value) -> pii_abi::RunEvent {
    pii_abi::RunEvent::NodeFinished {
        node: NodeId::from(node),
        port: PortName::from("ok"),
        value_hash: String::new(),
        value,
        at_ms: 0,
        dur_ms: 0,
    }
}
