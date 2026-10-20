//! Spec 234 R1, R2, R4: `skill.edit` changes a live skill's text in the daemon's own store, under the same id, and the
//! very next coder run sees it. A coder pinning the OLD sha256 refuses with `skill.drift`; the NEW one runs. The
//! controls are an id no live skill holds, a text that drops the body's `script:` pin, and a text with no marker:
//! each is refused and writes nothing. podman is a fake shim, as in `skill_put_through_the_daemon.rs`.

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
use pii_server::node_kinds::nodes::coder::{AGENT_BIN_DIR, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;
use sha2::{Digest, Sha256};

const SKILL: &str = "PII-SKILL\nlint-234\n# lint\nrun the linter\n";
const EDITED: &str = "PII-SKILL\nlint-234\n# lint\nrun the linter, then skip it\n";
const NO_MARKER: &str = "# lint\nrun the linter, unmarked\n";
const PINNED: &str = "PII-SKILL\nprobe-234\n# probe\nscript: scripts/probe.sh sha256=0123abcd\n";
const DROPS_PIN: &str = "PII-SKILL\nprobe-234\n# probe\nrun the probe, unpinned\n";
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
        name: "skill-edit-test".into(),
        version: "0".into(),
        build: "test".into(),
    }
}

/// What one coder leg left behind: the run's terminal, what it journalled, whether the container ran, and what the
/// fake podman was asked to do.
struct Leg {
    terminal: Terminal,
    events: Vec<Event>,
    ran: bool,
    journal: String,
}

/// A one-node container coder that pins `lint-234` at `pin` and runs against the engine's store.
fn coder_leg(engine: &Arc<Engine>, root: &Path, pin: &str) -> Leg {
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-edit-work-").unwrap();
    let shim =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-edit-podman-").unwrap();
    let podman = fake_podman(shim.path());
    let log = Arc::new(Rec::default());
    let reg = registry_over(engine, &podman, log.clone());
    let src = format!(
        "name = \"edit-skill\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/234-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"{IMAGE}\"\nskills = [{{ name = \"lint-234\", sha256 = \"{pin}\" }}]\n",
        work.path().display(),
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_store = pii_store::Store::open(&root.join("run.redb")).unwrap();
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
    Leg {
        terminal,
        events: log.0.lock().clone(),
        ran: work.path().join("ran.txt").exists(),
        journal: std::fs::read_to_string(shim.path().join("podman.log")).unwrap_or_default(),
    }
}

/// R1: the edit keeps the id, and answers the new slug and the sha256 a graph pin must move to.
fn assert_edit_reply(edit: &serde_json::Value, id: &str) {
    assert_eq!(edit["id"].as_str(), Some(id), "{edit}");
    assert_eq!(edit["slug"].as_str(), Some("lint-234"), "{edit}");
    assert_eq!(
        edit["sha256"].as_str(),
        Some(sha256_hex(EDITED).as_str()),
        "{edit}"
    );
    assert_eq!(edit["bytes_before"].as_u64(), Some(SKILL.len() as u64));
    assert_eq!(edit["bytes_after"].as_u64(), Some(EDITED.len() as u64));
}

/// R4: a coder still pinning the old text refuses before anything runs, and journals the drift.
fn assert_old_pin_refused(stale: &Leg) {
    let Terminal::Success { output } = &stale.terminal else {
        panic!(
            "a refusal ends the run with its reason: {:?}",
            stale.terminal
        );
    };
    let reason = output
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        reason.starts_with("skill.drift: lint-234 "),
        "the old pin refuses and names the skill: {reason}"
    );
    assert!(
        stale.events.iter().any(|e| e.kind == names::SKILL_DRIFT),
        "skill.drift is journalled"
    );
    assert!(!stale.ran, "a drifted skill must not reach the container");
    assert!(
        !stale.journal.lines().any(|l| l.starts_with("run ")),
        "the fake podman journal holds no run line: {}",
        stale.journal
    );
}

/// R4: the same graph pinned at the new text runs, and the spawn names the skill with the new hash.
fn assert_new_pin_spawns(fresh: &Leg) {
    assert!(
        matches!(fresh.terminal, Terminal::Success { .. }),
        "a pin that matches the edit runs: {:?}",
        fresh.terminal
    );
    assert!(fresh.ran, "the container ran");
    let spawned = fresh
        .events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned is journalled");
    assert_eq!(
        spawned.fields.get("skill").and_then(|v| v.as_str()),
        Some("lint-234"),
        "{:?}",
        spawned.fields
    );
    assert_eq!(
        spawned.fields.get("sha256").and_then(|v| v.as_str()),
        Some(sha256_hex(EDITED).as_str()),
        "{:?}",
        spawned.fields
    );
}

/// The stored texts after the refused edits: one live skill, the pinned original byte-equal, and the edited text
/// still in place under its id.
fn assert_refusals_wrote_nothing(engine: &Engine, id: &str, pinned_id: &str) {
    let live = pii_memory::skill::live(&engine.memory).unwrap();
    assert_eq!(
        live.len(),
        2,
        "the refused puts and edits added no skill: {live:?}"
    );
    let text_of = |want: &str| live.iter().find(|(i, _)| i == want).map(|(_, t)| t.clone());
    assert_eq!(
        text_of(pinned_id),
        Some(PINNED.to_owned()),
        "a dropped pin leaves the stored text byte-equal"
    );
    assert_eq!(text_of(id), Some(EDITED.to_owned()), "the edit held");
}

#[tokio::test]
async fn skill_edit_through_the_daemons_store() {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-edit-").unwrap();
    let engine = engine_in(dir.path());
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(&engine));
    let (r, w) = tokio::io::split(stream);
    let (client, _hello) = pii_proto::client::Client::connect(r, w, client_info())
        .await
        .expect("connect");

    let put = client
        .call(Method::SkillPut, serde_json::json!({ "text": SKILL }))
        .await
        .expect("skill.put");
    let id = put["id"].as_str().expect("put returns an id").to_owned();

    let edit = client
        .call(
            Method::SkillEdit,
            serde_json::json!({ "id": id, "text": EDITED }),
        )
        .await
        .expect("skill.edit");
    assert_edit_reply(&edit, &id);

    let stale = coder_leg(&engine, dir.path(), &sha256_hex(SKILL));
    assert_old_pin_refused(&stale);
    let fresh = coder_leg(&engine, dir.path(), &sha256_hex(EDITED));
    assert_new_pin_spawns(&fresh);

    // Controls, each a refusal that must write nothing.
    let pinned = client
        .call(Method::SkillPut, serde_json::json!({ "text": PINNED }))
        .await
        .expect("skill.put the pinned skill");
    let pinned_id = pinned["id"].as_str().expect("put returns an id").to_owned();
    let unknown = client
        .call(
            Method::SkillEdit,
            serde_json::json!({ "id": "no-such-skill", "text": EDITED }),
        )
        .await;
    assert!(unknown.is_err(), "an unknown id is refused: {unknown:?}");
    let dropped = client
        .call(
            Method::SkillEdit,
            serde_json::json!({ "id": pinned_id, "text": DROPS_PIN }),
        )
        .await;
    assert!(dropped.is_err(), "a dropped pin is refused: {dropped:?}");
    let bare = client
        .call(
            Method::SkillEdit,
            serde_json::json!({ "id": id, "text": NO_MARKER }),
        )
        .await;
    assert!(
        bare.is_err(),
        "a text without PII-SKILL is refused: {bare:?}"
    );
    assert_refusals_wrote_nothing(&engine, &id, &pinned_id);
}
