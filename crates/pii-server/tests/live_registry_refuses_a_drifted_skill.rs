//! Spec 227 R5: on the live registry, a stored skill whose text no longer hashes to its pin refuses the `coder` node
//! before any container argv is built. The store holds the document, then its bytes are edited under the same id.
//! The refusal names the skill, and the podman journal holds no `run` line. The control is the untouched store,
//! which spawns once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_memory::doc::{Doc, DocKind, Layer, rows_for};
use pii_model::ScriptedProvider;
use pii_observe::{Event, Observer, names};
use pii_server::engine::registry::registry;
use pii_server::node_kinds::nodes::coder::{AGENT_BIN_DIR, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::tiers::TierTable;
use pii_server::{DEFAULT_SESSION_RUNS_CAP, EngineConfig};
use pii_store::MemStore;
use pii_tools::sandbox::SandboxProfileName;
use sha2::{Digest, Sha256};

const STORED: &str = "PII-SKILL\nlint\n# lint\nrun the linter\n";
const EDITED: &str = "PII-SKILL\nlint\n# lint\nrun the linter, then skip it\n";
const IMAGE: &str = "localhost/pii-agent@sha256:feedface";

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A fake `podman` in `dir` that answers the image probe and journals each argv. `run` touches `ran.txt`.
fn fake_podman(dir: &Path) -> PathBuf {
    let log = dir.join("podman.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1\" in\n  image) echo sha256:feedface; exit 0 ;;\n  run) touch ./ran.txt; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        log.display(),
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::fs::Permissions::from_mode(0o755)).unwrap();
    podman
}

fn root_ctx(podman: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![
        String::from("/bin/sh"),
        podman.display().to_string(),
        String::from("/usr/bin/test"),
    ];
    argv0.extend(agent_binaries(Path::new(AGENT_BIN_DIR)));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// The daemon's registry, built the way `lib.rs` builds it, over the stores in `store_dir`.
fn live_registry(
    store_dir: &Path,
    memory: Arc<MemStore>,
    podman: &Path,
    log: Arc<Rec>,
) -> NodeRegistry {
    let cfg = EngineConfig {
        workspace: store_dir.to_path_buf(),
        store_path: store_dir.join("store.redb"),
        model: String::from("scripted:none"),
        sandbox: SandboxProfileName::Off,
        graph_dir: store_dir.join("graphs"),
        route_policy: BTreeMap::new(),
        grant_ceiling: None,
        tiers: Vec::new(),
        session_runs_cap: DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
    };
    let approvals = pii_store::approval::ApprovalStore::open(&store_dir.join("approval.redb"));
    let heals = pii_store::heal::HealStore::open(&store_dir.join("heal.redb"));
    registry(
        &cfg,
        Arc::new(ScriptedProvider::empty()),
        tokio::sync::broadcast::channel(4).0,
        Arc::new(approvals.unwrap()),
        Arc::new(heals.unwrap()),
        memory,
        Arc::new(Mutex::new(BTreeMap::new())),
        BTreeMap::new(),
        Arc::new(Mutex::new(None)),
        log,
        Arc::new(TierTable::build(&[], |_| unreachable!(), |_, _, _| {})),
        podman.to_path_buf(),
    )
}

/// Runs the one-node container graph that pins `lint` at `pin`, over a store where the skill is first stored as
/// `STORED`. When `edit` is set, the document's bytes are then replaced under the same id.
fn run_pinned(pin: &str, edit: bool) -> (Terminal, Vec<Event>, tempfile::TempDir, String) {
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "drift-work-").unwrap();
    let shim =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "drift-podman-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "drift-store-").unwrap();
    let podman = fake_podman(shim.path());
    let memory = Arc::new(MemStore::open(&store_dir.path().join("memory.redb")).unwrap());
    let doc = Doc::new(Layer::L3, DocKind::Skill, STORED, vec![], 1.0, 1_000);
    memory.put(&rows_for(&doc)).unwrap();
    if edit {
        // Same id, new bytes: the store now holds text the pin was not reviewed at.
        let mut edited = doc;
        EDITED.clone_into(&mut edited.text);
        memory.put(&rows_for(&edited)).unwrap();
    }
    let log = Arc::new(Rec::default());
    let reg = live_registry(store_dir.path(), memory, &podman, log.clone());

    let src = format!(
        "name = \"live-drift\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/227-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"{IMAGE}\"\nskills = [{{ name = \"lint\", sha256 = \"{pin}\" }}]\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_store = pii_store::Store::open(&store_dir.path().join("run.redb")).unwrap();
    let run_id = run_store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &run_store,
        &spec,
        &reg,
        &root_ctx(&podman),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let podman_log = std::fs::read_to_string(shim.path().join("podman.log")).unwrap_or_default();
    let events = log.0.lock().clone();
    (terminal, events, work, podman_log)
}

#[test]
fn a_drifted_stored_skill_refuses_on_the_live_path() {
    let (terminal, events, work, podman_log) = run_pinned(&sha256_hex(STORED), true);

    assert!(
        !work.path().join("ran.txt").exists(),
        "a drifted skill must not reach the container"
    );
    assert!(
        podman_log.lines().all(|l| !l.starts_with("run ")),
        "no container run was journalled: {podman_log}"
    );
    let Terminal::Success { output } = &terminal else {
        panic!("a refusal routes to err and ends the run with its reason: {terminal:?}");
    };
    let reason = output
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        reason.starts_with("skill.drift: lint "),
        "the refusal names the skill: {reason}"
    );
    assert!(
        events.iter().any(|e| e.kind == names::SKILL_DRIFT),
        "skill.drift is journalled"
    );
    assert!(events.iter().all(|e| e.kind != names::CODER_SPAWNED));
}

#[test]
fn an_untouched_stored_skill_spawns_once() {
    let (terminal, _events, work, podman_log) = run_pinned(&sha256_hex(STORED), false);

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "a matching pin runs: {terminal:?}"
    );
    assert!(work.path().join("ran.txt").exists(), "the container ran");
    assert_eq!(
        podman_log.lines().filter(|l| l.starts_with("run ")).count(),
        1,
        "exactly one run line: {podman_log}"
    );
}
