//! Spec 225 R3: a run's pinned skills are materialised into a per-run directory, outside the project, and bound
//! read-only over `.pi/skills` and `.claude/skills` in the container. Nothing is written into the project.
//! podman is a fake shim here (it is not installed in the test environment); it journals each argv so the test
//! can read the mounts the node asked for.

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
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::coder_skills::SkillSource;
use pii_server::node_kinds::nodes::criteria;
use sha2::{Digest, Sha256};

const IMAGE: &str = "localhost/pii-agent@sha256:feedface";
const BODY: &str = "# lint\nrun the linter\n";

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

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

/// A fake `podman` in `dir`. It journals each invocation's argv to `dir/podman.log`, answers `image inspect`
/// with a digest for `IMAGE`, and answers `run` by touching `ran.txt` in its cwd (the node's workdir).
fn fake_podman(dir: &Path) -> PathBuf {
    let log = dir.join("podman.log");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{log}'\ncase \"$1\" in\n  image) if [ \"$5\" = '{IMAGE}' ]; then echo sha256:feedface; exit 0; fi; echo 'no such image' >&2; exit 1 ;;\n  run) touch ./ran.txt; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        log = log.display(),
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::fs::Permissions::from_mode(0o755)).unwrap();
    podman
}

/// A scratch bin directory holding a fake `pi`; the container path never runs it on the host.
fn fake_bin() -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "mat-bin-").unwrap();
    let pi = dir.path().join("pi");
    std::fs::write(&pi, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
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

/// The first `lint/SKILL.md` under `runs`, and the `skills` directory it sits in.
fn materialised(runs: &Path) -> Option<PathBuf> {
    for run in std::fs::read_dir(runs).ok()?.flatten() {
        let skills = run.path().join("skills");
        if skills.join("lint/SKILL.md").is_file() {
            return Some(skills);
        }
    }
    None
}

#[test]
fn pinned_skills_are_materialised_read_only_and_bound_ro() {
    let bin = fake_bin();
    let shim = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "mat-podman-").unwrap();
    let podman = fake_podman(shim.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "mat-work-").unwrap();
    let runs = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "mat-runs-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "mat-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();

    let log = Arc::new(Rec::default());
    let skills = Skills(BTreeMap::from([(String::from("lint"), BODY.to_owned())]));
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
            .with_podman(podman.clone())
            .with_skills(Arc::new(skills))
            .with_runs_dir(runs.path().to_path_buf()),
    ));
    let src = format!(
        "name = \"materialise\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/225-x.md\"\nworkdir = \"{work}\"\ncontainer_image = \"{IMAGE}\"\nskills = [{{ name = \"lint\", sha256 = \"{pin}\" }}]\n",
        work = work.path().display(),
        pin = sha256_hex(BODY),
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin.path(), &podman),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );

    // The document is on disk for this run, outside the project, and carries its pin.
    let skills_dir = materialised(runs.path()).expect("the pinned skill is materialised");
    assert!(
        !skills_dir.starts_with(work.path()),
        "materialised outside the project"
    );
    let file = skills_dir.join("lint/SKILL.md");
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text, format!("sha256={}\n{BODY}", sha256_hex(BODY)));

    // Read-only: no write bit on the file or on its directories.
    for path in [&file, &skills_dir.join("lint"), &skills_dir] {
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o222, 0, "{} has no write bit", path.display());
    }

    // Bound read-only over both agents' skill dirs, and nothing was written into the project.
    let argv = std::fs::read_to_string(shim.path().join("podman.log")).unwrap();
    let dir = skills_dir.display();
    assert!(
        argv.contains(&format!("-v {dir}:/work/.pi/skills:ro")),
        "pi mount: {argv}"
    );
    assert!(
        argv.contains(&format!("-v {dir}:/work/.claude/skills:ro")),
        "claude mount: {argv}"
    );
    assert!(
        !work.path().join(".pi/skills/lint").exists(),
        "nothing written into the project"
    );

    let events = log.0.lock().clone();
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned is journalled");
    assert_eq!(
        spawned.fields.get("skill").and_then(|v| v.as_str()),
        Some("lint")
    );
}
