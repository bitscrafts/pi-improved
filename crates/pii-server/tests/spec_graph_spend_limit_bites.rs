//! Spec 282 (e): the tracked `graphs/spec.toml` declares the run's spend limit, and it bites the coder. The limit is read
//! through the loader the daemon uses, handed to `Budget::new`, and the FIRST claude run is capped by it: the cap is read
//! from the argv the fake claude recorded (`--max-budget-usd 1`), which is the live path and not the config text.

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
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx, NullObserver, TraceCtx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer};
use pii_server::node_kinds::nodes::coder::{CoderFactory, agent_binaries};
use pii_server::node_kinds::nodes::criteria;
use pii_server::node_kinds::nodes::queue::JudgeFactory;
use pii_store::Store;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

fn graphs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

/// A scratch bin directory with a fake `pi` that exits 0 and a fake `claude` that appends its argv to `argv.log` beside
/// itself and reports a zero cost, so nothing is spent.
fn fake_bin() -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "spend-bin-").unwrap();
    let claude = concat!(
        "#!/bin/sh\n",
        "echo \"$*\" >> \"${0%/*}/argv.log\"\n",
        "printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"num_turns\":1,",
        "\"total_cost_usd\":0,\"result\":\"done\"}'\n",
    );
    for (name, script) in [
        ("pi", String::from("#!/bin/sh\nexit 0\n")),
        ("claude", claude.to_owned()),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    dir
}

fn registry(bin: &Path, log: Arc<Rec>) -> NodeRegistry {
    let mut reg = NodeRegistry::builtin();
    let env = BTreeMap::from([(
        String::from("CLAUDE_CODE_OAUTH_TOKEN"),
        String::from("placeholder"),
    )]);
    reg.register(Arc::new(CoderFactory::with_env(
        bin.to_path_buf(),
        env,
        log,
    )));
    reg.register(Arc::new(JudgeFactory));
    reg
}

/// The run budget the tracked graph declares: its `[limits]`, loaded the way a run loads its host.
fn tracked_limit() -> u64 {
    let src = std::fs::read_to_string(graphs_dir().join("spec.toml")).unwrap();
    let reg = registry(Path::new("/nonexistent-bin"), Arc::new(Rec::default()));
    pii_graph::fragment::load_host_with_graphs(&src, &graphs_dir(), &[graphs_dir()], &reg)
        .unwrap()
        .spec
        .limits
        .max_micro_usd
}

/// One coder node of agent `claude`. Its budget is the tracked limit handed to `Budget::new`, not a `[limits]` here.
fn one_coder(work: &Path) -> String {
    format!(
        "name = \"spend\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ntimeout_ms = 60000\ncaps = [\"spawn\", \"proc\"]\nagent = \"claude\"\nmodel = \"m-one\"\nbrief = \"do it\"\nspec = \"specs/282-x.md\"\nworkdir = \"{}\"\nenv_pass = [\"CLAUDE_CODE_OAUTH_TOKEN\"]\n",
        work.display()
    )
}

/// The `--max-budget-usd` value in one argv line, when the line carries the flag.
fn cap_flag(line: &str) -> Option<String> {
    line.split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[0] == "--max-budget-usd")
        .map(|w| w[1].to_owned())
}

#[test]
fn the_tracked_limit_caps_the_first_claude_run() {
    let limit = tracked_limit();
    let bin = fake_bin();
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "spend-work-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "spend-store-").unwrap();
    let store = Store::open(&store_dir.path().join("store.redb")).unwrap();
    let reg = registry(bin.path(), Arc::new(Rec::default()));
    let spec = pii_graph::load_toml_with_registry(&one_coder(work.path()), &reg).unwrap();
    let run_id = store.next_run_id().unwrap();

    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin.path()));
    argv0.extend(criteria::programs(&BTreeMap::new()));
    caps.proc_argv0 = ArgvScope::new(argv0);
    let trace = TraceCtx {
        run: run_id.clone(),
        span: 0,
    };
    let ctx = Ctx::with_observer(
        caps,
        Budget::new(u64::MAX, u64::MAX, limit, u64::MAX),
        Arc::new(NullObserver),
        trace,
    );
    pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &ctx,
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();

    let argv = std::fs::read_to_string(bin.path().join("argv.log")).unwrap_or_default();
    let first = argv.lines().next().unwrap_or_default();
    assert_eq!(
        cap_flag(first).as_deref(),
        Some("1"),
        "the first claude run is capped at the tracked 1.00 USD: {argv:?}"
    );
}
