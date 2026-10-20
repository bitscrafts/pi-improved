//! Spec 221 R1 and R3: a run whose graph ran a `coder` node succeeds only when that node's `coder.spawned` and
//! `coder.exited` are in the journal for the same pid. A node that reports success without a call is a failure,
//! whatever the run's duration: the journal is the only evidence, never the node's own output.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::Effects;
use pii_abi::{NodeInput, Outcome, PortName, RunStatus, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::{GraphNode, NodeFactory, NodeRegistry};
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A `coder` node that reports success and never makes an agent call: the defect spec 221 closes.
struct SilentCoder;

struct SilentNode;

impl NodeFactory for SilentCoder {
    fn kind(&self) -> &'static str {
        "coder"
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, pii_abi::PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, pii_abi::PiiError> {
        Ok(Box::new(SilentNode))
    }
}

impl GraphNode for SilentNode {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, pii_abi::PiiError> {
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({ "claimed": "done" }),
        })
    }
}

/// A root context with SPAWN + PROC and the run scope's programs, as the real coder tests grant them.
fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::from([(
        "green".to_owned(),
        vec!["/bin/true".to_owned()],
    )])));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs `src` through the real scheduler with `reg`, then returns the stored header and the terminal.
fn run_graph(
    reg: &NodeRegistry,
    src: &str,
    bin: &Path,
) -> (RunStatus, Terminal, pii_store::Store, tempfile::TempDir) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let spec = pii_graph::load_toml_with_registry(src, reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        reg,
        &root_ctx(bin),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let header = store.get_run(&run_id).unwrap().expect("the run is stored");
    (header.status, terminal, store, store_dir)
}

const SILENT_GRAPH: &str = "name = \"silent\"\nentry = \"code\"\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\n\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n";

/// A `coder` node that claims success without a `coder.spawned` makes the run FAILED, and the failure names the
/// node and the missing reason. Its duration is irrelevant: the claim alone is not evidence.
#[test]
fn silent_coder_node_cannot_succeed_the_run() {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(SilentCoder));
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();

    let (status, terminal, _store, _dir) = run_graph(&reg, SILENT_GRAPH, bin.path());

    assert_eq!(status, RunStatus::Failed, "a call-less coder run is failed");
    match terminal {
        Terminal::Failure { error } => {
            assert!(error.contains("coder.no_call_events"), "{error}");
            assert!(
                error.contains("code"),
                "the failure names the node: {error}"
            );
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

/// A coder node that really spawns its agent and exits, with a passing criterion, leaves a run SUCCEEDED.
#[test]
fn spawned_and_exited_coder_run_succeeds() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    std::fs::write(bin.path().join("pi"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(
        bin.path().join("pi"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log).with_criteria(
            BTreeMap::from([("green".to_owned(), vec!["/bin/true".to_owned()])]),
        ),
    ));
    let src = format!(
        "name = \"real\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/221-x.md\"\nworkdir = \"{}\"\ncriteria = [\"green\"]\n",
        work.path().display()
    );

    let (status, terminal, _store, _dir) = run_graph(&reg, &src, bin.path());

    assert!(matches!(terminal, Terminal::Success { .. }), "{terminal:?}");
    assert_eq!(status, RunStatus::Succeeded);
}
