//! Spec 227 R2, R4: the daemon's own registry hands a `coder` node its skill store. The test builds the node kind
//! through `registry(...)`, the function the daemon calls, over a memory store holding one pinned skill, and reads
//! what the live path did: the materialised file, the podman argv and the journalled `coder.spawned`. The control is
//! the same graph over a store without the skill: the node refuses as missing and no container run starts.
//! podman is a fake shim (it is not installed in the test environment); the container path never runs a host agent.

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
use pii_server::node_kinds::nodes::coder::{agent_bin_dir, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::tiers::TierTable;
use pii_server::{DEFAULT_SESSION_RUNS_CAP, EngineConfig};
use pii_store::MemStore;
use pii_tools::sandbox::SandboxProfileName;
use sha2::{Digest, Sha256};

const SKILL_TEXT: &str = "PII-SKILL\nlint\n# lint\nrun the linter\n";
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

/// A fake `podman` in `dir` that answers the image probe and journals each argv. `run` touches `ran.txt` in the
/// node's workdir, so a run that reaches the container leaves a marker.
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
    argv0.extend(agent_binaries(
        agent_bin_dir()
            .as_deref()
            .expect("agent binaries (pi, claude) on PATH"),
    ));
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

/// Stores `text` as a skill document in `memory`, as `pii skill` would find it.
fn store_skill(memory: &MemStore, text: &str) -> Doc {
    let doc = Doc::new(Layer::L3, DocKind::Skill, text, vec![], 1.0, 1_000);
    memory.put(&rows_for(&doc)).unwrap();
    doc
}

/// What one live run left behind.
struct Run {
    terminal: Terminal,
    events: Vec<Event>,
    work: tempfile::TempDir,
    store_dir: tempfile::TempDir,
    podman_log: String,
}

/// Runs a one-node container `coder` that pins `lint` at `pin`, against a store holding `stored` when it is `Some`.
fn run_pinned(stored: Option<&str>, pin: &str) -> Run {
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "live-work-").unwrap();
    let shim = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "live-podman-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "live-store-").unwrap();
    let podman = fake_podman(shim.path());
    let memory = Arc::new(MemStore::open(&store_dir.path().join("memory.redb")).unwrap());
    if let Some(text) = stored {
        store_skill(&memory, text);
    }
    let log = Arc::new(Rec::default());
    let reg = live_registry(store_dir.path(), memory, &podman, log.clone());

    let src = format!(
        "name = \"live-skill\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/227-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"{IMAGE}\"\nskills = [{{ name = \"lint\", sha256 = \"{pin}\" }}]\n",
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
    Run {
        terminal,
        events,
        work,
        store_dir,
        podman_log,
    }
}

/// The first `lint/SKILL.md` under the runs directory of `store_dir`, and the `skills` directory it sits in.
fn materialised(store_dir: &Path) -> Option<PathBuf> {
    let runs = store_dir.join("runs");
    for run in std::fs::read_dir(runs).ok()?.flatten() {
        let skills = run.path().join("skills");
        if skills.join("lint/SKILL.md").is_file() {
            return Some(skills);
        }
    }
    None
}

#[test]
fn the_live_registry_materialises_and_names_the_stored_skill() {
    let run = run_pinned(Some(SKILL_TEXT), &sha256_hex(SKILL_TEXT));
    assert!(
        matches!(run.terminal, Terminal::Success { .. }),
        "a matching pin runs: {:?}",
        run.terminal
    );
    assert!(
        run.work.path().join("ran.txt").exists(),
        "the container ran"
    );

    // (a) the file is under the store's runs dir, and not in the project.
    let skills_dir = materialised(run.store_dir.path()).expect("the skill is materialised");
    assert!(
        !skills_dir.starts_with(run.work.path()),
        "materialised outside the project"
    );
    assert!(!run.work.path().join(".pi/skills/lint").exists());

    // (b) the bytes are the stored document, behind the sha256 header the materialiser writes.
    let file = skills_dir.join("lint/SKILL.md");
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        text,
        format!("sha256={}\n{SKILL_TEXT}", sha256_hex(SKILL_TEXT))
    );

    // (d) both agents' skill dirs are bound read-only from that directory.
    let dir = skills_dir.display();
    assert!(
        run.podman_log
            .contains(&format!("-v {dir}:/work/.pi/skills:ro")),
        "pi mount: {}",
        run.podman_log
    );
    assert!(
        run.podman_log
            .contains(&format!("-v {dir}:/work/.claude/skills:ro")),
        "claude mount: {}",
        run.podman_log
    );

    // (c) the journalled spawn names the skill and carries its pin.
    let spawned = run
        .events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned is journalled");
    assert_eq!(
        spawned.fields.get("skill").and_then(|v| v.as_str()),
        Some("lint")
    );
    assert_eq!(
        spawned.fields.get("sha256").and_then(|v| v.as_str()),
        Some(sha256_hex(SKILL_TEXT).as_str())
    );
}

#[test]
fn a_store_without_the_skill_refuses_as_missing() {
    // The control: the same graph, the same pin, and no skill in the store. Green (a)-(d) must not come from
    // the registry ignoring its store.
    let run = run_pinned(None, &sha256_hex(SKILL_TEXT));
    let Terminal::Success { output } = &run.terminal else {
        panic!(
            "a refusal routes to err and ends the run with its reason: {:?}",
            run.terminal
        );
    };
    let reason = output
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        reason.starts_with("skill.drift: lint ") && reason.ends_with("found missing"),
        "a missing skill refuses: {reason}"
    );
    assert!(
        run.podman_log.lines().all(|l| !l.starts_with("run ")),
        "no container run was journalled: {}",
        run.podman_log
    );
    assert!(!run.work.path().join("ran.txt").exists());
}
