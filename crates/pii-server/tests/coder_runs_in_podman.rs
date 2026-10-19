//! Spec 209: a `coder` node with `container_image` runs its agent inside a prebuilt image through podman. The
//! agent binary is never a host argv; the image is graph data; the host path is a deprecated fallback that
//! announces itself. podman is a fake shim here (it is not installed in the test environment), and the shim
//! journals every invocation so the tests can read what the node asked for.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::Terminal;
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A fake `podman` in `dir`. It journals each invocation's argv to `dir/podman.log`, answers `image inspect`
/// with a digest for every image named in `images`, and answers `run` by touching `ran.txt` in its cwd (the
/// node's workdir), standing in for the agent having run inside the container.
fn fake_podman(dir: &Path, images: &[&str]) -> PathBuf {
    let log = dir.join("podman.log");
    let names_file = dir.join("images");
    std::fs::write(&names_file, images.join("\n")).unwrap();
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{log}'\ncase \"$1\" in\n  image) if grep -qx -- \"$5\" '{images}'; then echo sha256:feedface; exit 0; fi; echo 'no such image' >&2; exit 1 ;;\n  run) touch ./ran.txt; exit 0 ;;\n  *) exit 2 ;;\nesac\n",
        log = log.display(),
        images = names_file.display(),
    );
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

/// A root context with SPAWN + PROC and the run scope's programs: `/bin/sh`, the agent binaries, and podman.
fn root_ctx(bin: &Path, podman: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh"), podman.display().to_string()];
    argv0.extend(agent_binaries(bin));
    caps.proc_argv0 = ArgvScope::new(argv0);
    Ctx::root(caps, Budget::unlimited())
}

/// Runs a one-node `coder` graph through the real scheduler and returns the terminal and the events.
fn run_coder_graph(
    bin: &Path,
    podman: &Path,
    node_fields: &str,
    env: BTreeMap<String, String>,
) -> (Terminal, Vec<Event>) {
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.to_path_buf(), env, log.clone())
            .with_podman(podman.to_path_buf()),
    ));
    let src = format!(
        "name = \"coder-podman\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\n{node_fields}\n"
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let run_id = store.next_run_id().unwrap();
    let terminal = pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &root_ctx(bin, podman),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();
    let events = log.0.lock().clone();
    (terminal, events)
}

/// The journalled invocations of the fake podman, one per line.
fn podman_log(podman: &Path) -> Vec<String> {
    std::fs::read_to_string(podman.parent().unwrap().join("podman.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The `run` invocation (the one that starts the agent), which must exist exactly once.
fn run_line(lines: &[String]) -> String {
    let runs: Vec<&String> = lines.iter().filter(|l| l.starts_with("run ")).collect();
    assert_eq!(runs.len(), 1, "exactly one container run: {lines:?}");
    runs[0].clone()
}

/// A coder node whose agent runs in `image`; the workdir is `work`, the env pass is `PII_TEST_KEY`.
fn container_fields(image: &str, work: &Path) -> String {
    format!(
        "agent = \"pi\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/209-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"{image}\"\nenv_pass = [\"PII_TEST_KEY\"]\ncriteria = [\"test -f ran.txt\"]\n",
        work.display()
    )
}

/// The agent runs inside the image: the `run` argv begins `run --rm`, mounts only the workspace, has no network
/// by default, and carries the image as graph data. The host agent binary is never on that argv, and the
/// env-pass value never appears in any argv.
#[test]
fn coder_runs_in_podman() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman(bin.path(), &["pii-coder:test"]);
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let env = BTreeMap::from([(
        String::from("PII_TEST_KEY"),
        String::from("secret-value-xyz"),
    )]);

    let (terminal, events) = run_coder_graph(
        bin.path(),
        &podman,
        &container_fields("pii-coder:test", work.path()),
        env,
    );

    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    assert!(
        work.path().join("ran.txt").exists(),
        "the agent ran in the container"
    );
    let lines = podman_log(&podman);
    let run = run_line(&lines);
    assert!(run.starts_with("run --rm --workdir /work"), "{run}");
    assert!(
        run.contains(&format!("-v {}:/work", work.path().display())),
        "{run}"
    );
    assert!(
        run.contains("--network none"),
        "no network by default: {run}"
    );
    assert!(
        run.contains("pii-coder:test pi "),
        "the image is the graph's: {run}"
    );
    assert!(
        !run.contains(&bin.path().join("pi").display().to_string()),
        "no host agent binary in argv: {run}"
    );
    assert!(
        lines.iter().all(|l| !l.contains("secret-value-xyz")),
        "an env value never reaches an argv: {lines:?}"
    );
    let spawned = events
        .iter()
        .find(|e| e.kind == names::CODER_SPAWNED)
        .expect("coder.spawned journalled");
    assert_eq!(
        spawned
            .fields
            .get("image_digest")
            .and_then(serde_json::Value::as_str),
        Some("sha256:feedface"),
        "the journal names the image digest"
    );
}

/// The image is graph data: two graphs naming two images produce two different `run` argvs for the same agent.
#[test]
fn image_is_graph_data() {
    let mut runs = Vec::new();
    for image in ["pii-coder:a", "pii-coder:b"] {
        let bin =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
        let podman = fake_podman(bin.path(), &[image]);
        let work =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
        let (terminal, _) = run_coder_graph(
            bin.path(),
            &podman,
            &container_fields(image, work.path()),
            BTreeMap::from([(String::from("PII_TEST_KEY"), String::from("v"))]),
        );
        assert!(
            matches!(terminal, Terminal::Success { .. }),
            "got {terminal:?}"
        );
        runs.push(run_line(&podman_log(&podman)));
    }
    assert_ne!(runs[0], runs[1]);
    assert!(
        runs[0].contains("pii-coder:a pi ") && runs[1].contains("pii-coder:b pi "),
        "{runs:?}"
    );
}

/// The host path is a deprecated fallback: a coder node with no `container_image` still runs, but journals
/// `coder.host_spawn.deprecated` so the host dependency is visible in the run's record.
#[test]
fn host_path_announces_itself() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    std::fs::write(bin.path().join("pi"), "#!/bin/sh\ntouch \"$PWD/ran.txt\"\n").unwrap();
    std::fs::set_permissions(
        bin.path().join("pi"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let podman = bin.path().join("podman");
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let fields = format!(
        "agent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/209-x.md\"\nworkdir = \"{}\"\ncriteria = [\"test -f ran.txt\"]\n",
        work.path().display()
    );
    let (terminal, events) = run_coder_graph(bin.path(), &podman, &fields, BTreeMap::new());
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "got {terminal:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e.kind == names::CODER_HOST_SPAWN_DEPRECATED),
        "host spawn journals its deprecation"
    );
}
