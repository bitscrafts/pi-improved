//! Spec 207: the run scope admits the agent binary for a `coder` node, and only for it. A node of any other
//! kind that execs the same argv is denied.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{CapKind, Terminal};
use pii_core::{ArgvScope, Budget, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_graph::sched::dispatch::node_proc_argv0;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{AGENT_BIN_DIR, CoderFactory, agent_binaries};

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// The run scope's programs: `/bin/sh` and the agent binaries under `bin` (as `run_caps` grants them).
fn root_caps(bin: &std::path::Path) -> CapSet {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin));
    caps.proc_argv0 = ArgvScope::new(argv0);
    caps
}

/// A coder node reaches its agent through the scheduler's node scope: the run completes and the agent ran.
#[test]
fn coder_node_scope_admits_its_agent_binary() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let pi = bin.path().join("pi");
    std::fs::write(&pi, "#!/bin/sh\necho ran > \"$PWD/ran.txt\"\n").unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();

    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(CoderFactory::with_env(
        bin.path().to_path_buf(),
        BTreeMap::new(),
        log.clone(),
    )));
    let src = format!(
        "name = \"coder-scope\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/207-x.md\"\nworkdir = \"{}\"\ncriteria = [\"test -f ran.txt\"]\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let root = Ctx::root(root_caps(bin.path()), Budget::unlimited());
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    assert!(work.path().join("ran.txt").exists());
    assert!(log.0.lock().iter().any(|e| e.kind == names::CODER_SPAWNED));
}

/// The program scope a non-coder node gets is the run's minus the agent binaries; a coder node keeps them. The
/// denial is checked at the spawn gate itself, so nothing is ever executed.
#[test]
fn non_coder_node_cannot_exec_the_agent_binary() {
    let bin = std::path::PathBuf::from(AGENT_BIN_DIR);
    let run_scope = ArgvScope::new({
        let mut v = vec![String::from("/bin/sh")];
        v.extend(agent_binaries(&bin));
        v
    });
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(CoderFactory::with_env(
        bin,
        BTreeMap::new(),
        Arc::new(Rec::default()),
    )));
    let agent_pi = format!("{AGENT_BIN_DIR}/pi");
    let argv = vec![agent_pi.clone(), String::from("-p")];
    let spawn = |scope: ArgvScope| {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::SPAWN | CapKind::PROC;
        caps.proc_argv0 = scope;
        Ctx::root(caps, Budget::unlimited())
            .proc()
            .unwrap()
            .spawn_child(&argv, std::path::Path::new("/tmp"), &[])
    };

    let non_coder = node_proc_argv0(&reg, "transform", &run_scope);
    let err = spawn(non_coder).expect_err("a non-coder node must be denied");
    assert!(matches!(err, pii_abi::PiiError::Denied(_)), "got {err:?}");

    let coder = node_proc_argv0(&reg, "coder", &run_scope);
    assert!(
        coder.permits(&agent_pi),
        "the coder kind keeps its agent binary"
    );
}
