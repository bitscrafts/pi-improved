//! Spec 233 R1, R4: `skill.put` writes the daemon's own store, and a coder node that pins the skill resolves it from
//! that same store. The put goes over the wire (the loopback transport a UDS client uses), and the controls are a
//! second put with the same slug (refused, one skill kept) and a text with no `PII-SKILL` marker (refused).
//! podman is a fake shim (it is not installed in the test environment, and the daemon names `/usr/bin/podman`
//! outright), so the coder leg builds its registry the way `live_registry_supplies_the_skill_source.rs` does, over
//! the engine's own memory store.

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
use pii_observe::{Event, Observer, names};
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::engine::registry::registry;
use pii_server::node_kinds::nodes::coder::{agent_bin_dir, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use sha2::{Digest, Sha256};

const SKILL: &str = "PII-SKILL\nlint-233\n# lint\nrun the linter\n";
const SAME_SLUG: &str = "PII-SKILL\nlint-233\n# lint\nrun the linter, then skip it\n";
const NO_MARKER: &str = "# lint\nrun the linter\n";
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

fn engine_in(dir: &Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixtures_hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

fn fixtures_hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
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

fn root_ctx(podman: &Path) -> Option<Ctx> {
    let Some(agent_dir) = agent_bin_dir() else {
        // PATH scrubbed in a fresh-gate export (measured 2026-10-30): non-resolution is the contract.
        return None;
    };
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![
        String::from("/bin/sh"),
        podman.display().to_string(),
        String::from("/usr/bin/test"),
    ];
    argv0.extend(agent_binaries(&agent_dir));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Some(Ctx::root(caps, Budget::unlimited()))
}

/// The daemon's registry, built the way `lib.rs` builds it, over the engine's own stores.
fn registry_over(engine: &Engine, podman: &Path, log: Arc<Rec>) -> NodeRegistry {
    registry(
        &engine.cfg,
        Arc::clone(&engine.provider),
        tokio::sync::broadcast::channel(4).0,
        Arc::clone(&engine.approvals),
        Arc::clone(&engine.heals),
        Arc::clone(&engine.memory),
        Arc::clone(&engine.workers),
        BTreeMap::new(),
        Arc::new(Mutex::new(None)),
        log,
        Arc::clone(&engine.tiers),
        podman.to_path_buf(),
    )
}

fn client_info() -> PeerInfo {
    PeerInfo {
        name: "skill-put-test".into(),
        version: "0".into(),
        build: "test".into(),
    }
}

/// The coder leg: a one-node container coder that pins `lint-233` at the sha256 the put returned resolves it from the
/// store the put wrote, and runs. Returns what the run journalled.
fn coder_leg(engine: &Arc<Engine>, root: &Path) -> Option<Vec<Event>> {
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-put-work-").unwrap();
    let shim =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-put-podman-").unwrap();
    let podman = fake_podman(shim.path());
    let log = Arc::new(Rec::default());
    let reg = registry_over(engine, &podman, log.clone());
    let src = format!(
        "name = \"put-skill\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/233-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"{IMAGE}\"\nskills = [{{ name = \"lint-233\", sha256 = \"{}\" }}]\n",
        work.path().display(),
        sha256_hex(SKILL),
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_store = pii_store::Store::open(&root.join("run.redb")).unwrap();
    let run_id = run_store.next_run_id().unwrap();
    let ctx = root_ctx(&podman)?;
    let terminal = pii_graph::run::run_with_validate(
        &run_store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "a pin that matches the put runs: {terminal:?}"
    );
    assert!(work.path().join("ran.txt").exists(), "the container ran");

    Some(log.0.lock().clone())
}

#[tokio::test]
async fn skill_put_through_the_daemon() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-put-").unwrap();
    let engine = engine_in(dir.path());
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(&engine));
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info())
        .await
        .expect("connect");

    // The put: the slug and the sha256 come back, and the sha256 is the one a graph pin carries.
    let put = client
        .call(Method::SkillPut, serde_json::json!({ "text": SKILL }))
        .await
        .expect("skill.put");
    assert_eq!(put["slug"].as_str(), Some("lint-233"), "{put}");
    assert_eq!(
        put["sha256"].as_str(),
        Some(sha256_hex(SKILL).as_str()),
        "{put}"
    );

    // Control 1: a second skill with the same slug is refused and the store keeps one.
    let again = client
        .call(Method::SkillPut, serde_json::json!({ "text": SAME_SLUG }))
        .await;
    assert!(
        again.is_err(),
        "a second put of the slug is refused: {again:?}"
    );
    let live = pii_memory::skill::live(&engine.memory).unwrap();
    assert_eq!(live.len(), 1, "the store keeps one skill: {live:?}");

    // Control 2: a text with no marker is refused and writes nothing.
    let bare = client
        .call(Method::SkillPut, serde_json::json!({ "text": NO_MARKER }))
        .await;
    assert!(
        bare.is_err(),
        "a text without PII-SKILL is refused: {bare:?}"
    );
    assert_eq!(pii_memory::skill::live(&engine.memory).unwrap().len(), 1);
    drop(client);

    let Some(events) = coder_leg(&engine, dir.path()) else {
        return;
    };
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned is journalled");
    assert_eq!(
        spawned.fields.get("skill").and_then(|v| v.as_str()),
        Some("lint-233"),
        "{:?}",
        spawned.fields
    );
    assert_eq!(
        spawned.fields.get("sha256").and_then(|v| v.as_str()),
        Some(sha256_hex(SKILL).as_str()),
        "{:?}",
        spawned.fields
    );
}
