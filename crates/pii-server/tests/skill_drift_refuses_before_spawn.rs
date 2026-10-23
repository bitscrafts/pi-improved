//! Spec 225 R4, R7: a `coder` node that pins a skill refuses before it spawns when the stored document no longer
//! hashes to the pin, and journals `skill.drift`. A matching pin proceeds, and `coder.spawned` names the skill.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::coder_skills::SkillSource;
use pii_server::node_kinds::nodes::criteria;
use sha2::{Digest, Sha256};

const STORED: &str = "# lint\nrun the linter\n";
const EDITED: &str = "# lint\nrun the linter, then skip it\n";

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// The skill store the test holds: name to the document text it returns.
struct Skills(BTreeMap<String, String>);

impl SkillSource for Skills {
    fn skill_text(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
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
fn fake_podman(dir: &Path) -> std::path::PathBuf {
    let log = dir.join("podman.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1\" in\n  image) echo sha256:feedface; exit 0 ;;\n  run) touch ./ran.txt; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        log.display(),
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

fn root_ctx(bin: &Path, podman: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![
        String::from("/bin/sh"),
        podman.display().to_string(),
        String::from("/usr/bin/test"),
    ];
    argv0.extend(agent_binaries(bin));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs a one-node container `coder` graph that pins `pin` for skill `lint`, against a store holding `stored_text`.
fn run_pinned(pin: &str, stored_text: &str) -> (Terminal, Vec<Event>, tempfile::TempDir) {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-bin-").unwrap();
    let shim =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-podman-").unwrap();
    let podman = fake_podman(shim.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-work-").unwrap();
    let runs = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-runs-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "skill-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let skills = Skills(BTreeMap::from([(
        String::from("lint"),
        stored_text.to_owned(),
    )]));
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
            .with_podman(podman.clone())
            .with_skills(Arc::new(skills))
            .with_runs_dir(runs.path().to_path_buf()),
    ));
    let src = format!(
        "name = \"skill-pin\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/225-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"localhost/pii-agent@sha256:feedface\"\nskills = [{{ name = \"lint\", sha256 = \"{pin}\" }}]\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let store_run = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin.path(), &podman),
        &store_run,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let events = log.0.lock().clone();
    (terminal, events, work)
}

#[test]
fn a_drifted_skill_refuses_before_the_agent_spawns() {
    // The graph pinned the text the store held when it was reviewed; the store has since been edited.
    let (terminal, events, work) = run_pinned(&sha256_hex(STORED), EDITED);

    assert!(
        !work.path().join("ran.txt").exists(),
        "a drifted skill must not reach the container"
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
        "refusal names the drift: {reason}"
    );
    let drift = events
        .iter()
        .find(|e| e.kind == names::SKILL_DRIFT)
        .expect("skill.drift is journalled");
    assert_eq!(
        drift.fields.get("name").and_then(|v| v.as_str()),
        Some("lint")
    );
    assert!(
        events.iter().all(|e| e.kind != names::CODER_SPAWNED),
        "no coder.spawned for a refused run"
    );
}

#[test]
fn a_matching_pin_proceeds_and_the_spawn_names_the_skill() {
    let (terminal, events, work) = run_pinned(&sha256_hex(EDITED), EDITED);

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "a matching pin runs: {terminal:?}"
    );
    assert!(work.path().join("ran.txt").exists(), "the container ran");
    assert!(events.iter().all(|e| e.kind != names::SKILL_DRIFT));
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned is journalled");
    assert_eq!(
        spawned.fields.get("skill").and_then(|v| v.as_str()),
        Some("lint")
    );
    assert_eq!(
        spawned.fields.get("sha256").and_then(|v| v.as_str()),
        Some(sha256_hex(EDITED).as_str())
    );
}
