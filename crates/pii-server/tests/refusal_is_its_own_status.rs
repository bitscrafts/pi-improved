//! Spec 221 R2 and R5: a refused agent call makes its run `refused`, a status distinct from `failed` and from
//! `succeeded`. A graph that declares `expect_spawn = false` is refused when its coder node makes no call, and the
//! run says so. A call that spawned and then broke stays `failed`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use pii_abi::Effects;
use pii_abi::{NodeInput, Outcome, PiiError, PortName, RunStatus, Terminal};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::{GraphNode, NodeFactory, NodeRegistry};
use pii_observe::Observer;
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;

struct Quiet;

impl Observer for Quiet {
    fn event(&self, _e: &pii_observe::Event) {}
}

/// A scratch bin directory holding a fake `pi` whose body is `body` (a POSIX shell snippet).
fn fake_bin(body: &str) -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    dir
}

fn root_ctx(bin: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&criteria_table()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

fn criteria_table() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        ("green".to_owned(), vec!["/bin/true".to_owned()]),
        ("red".to_owned(), vec!["/bin/false".to_owned()]),
    ])
}

/// Runs `src` with a real coder registry over `bin` and returns the stored status and the terminal.
fn run_status(bin: &Path, src: &str) -> (RunStatus, Terminal) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.to_path_buf(), BTreeMap::new(), Arc::new(Quiet))
            .with_criteria(criteria_table()),
    ));
    let spec = pii_graph::load_toml_with_registry(src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let header = store.get_run(&run_id).unwrap().expect("the run is stored");
    (header.status, terminal)
}

/// A coder node whose agent has no credentials is refused: it routes to `err`, the run completes, and the run is
/// REFUSED, not succeeded.
#[test]
fn refused_agent_call_makes_the_run_refused() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let src = format!(
        "name = \"refused\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"claude\"\nmodel = \"haiku-5-5\"\nbrief = \"b\"\nspec = \"specs/221-x.md\"\nworkdir = \"{}\"\ncriteria = [\"green\"]\n\n[[node]]\nid = \"refused\"\nkind = \"transform\"\nvalue = \"refused\"\n\n[[edge]]\nfrom = \"code\"\nport = \"err\"\nto = \"refused\"\nkind = \"data\"\n",
        work.path().display()
    );

    let (status, _terminal) = run_status(bin.path(), &src);

    assert_eq!(
        status,
        RunStatus::Refused,
        "a refused call is a refused run"
    );
    assert_ne!(status, RunStatus::Failed);
    assert_ne!(status, RunStatus::Succeeded);
}

/// A `coder` node that claims success and makes no call, standing in for a demo that refuses on purpose.
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
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(SilentNode))
    }
}

impl GraphNode for SilentNode {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({}),
        })
    }
}

/// `expect_spawn = false` on a coder node that makes no call makes the run REFUSED, not failed: the graph says the
/// refusal is deliberate, and `run get` names it.
#[test]
fn expect_spawn_false_refuses_a_call_less_run() {
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(SilentCoder));
    let src = "name = \"demo\"\nentry = \"code\"\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\nexpect_spawn = false\n\n[[node]]\nid = \"done\"\nkind = \"transform\"\nvalue = \"done\"\n\n[[edge]]\nfrom = \"code\"\nport = \"ok\"\nto = \"done\"\nkind = \"data\"\n";
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let spec = pii_graph::load_toml_with_registry(src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let bin = fake_bin("exit 0");
    pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin.path()),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let header = store.get_run(&run_id).unwrap().expect("the run is stored");

    assert_eq!(
        header.status,
        RunStatus::Refused,
        "the deliberate refusal is refused"
    );
}

/// A call that spawned and exited and then failed its criterion is FAILED: it started and broke, which is not a
/// refusal.
#[test]
fn a_call_that_started_and_broke_stays_failed() {
    let bin = fake_bin("exit 0");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let src = format!(
        "name = \"broke\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/221-x.md\"\nworkdir = \"{}\"\ncriteria = [\"red\"]\n",
        work.path().display()
    );

    let (status, terminal) = run_status(bin.path(), &src);

    assert_eq!(status, RunStatus::Failed, "{terminal:?}");
}
