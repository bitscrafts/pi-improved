//! Spec 363: `plan` writes one new spec under `specs/` from a finding, and `enqueue` admits it only when the spec has the
//! house shape. The planner is a stub model that returns a scripted reply; the validators that judge the written spec
//! are the lane's own `row-verify.py` and `row-paths.py`, run on the row the enqueue node produced.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods // reason: test fixture only; the validators are the lane's own scripts, run as the lane runs them
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome};
use pii_core::{Budget, CapKind, CapSet, Ctx, PathScope};
use pii_graph::node::NodeFactory;
use pii_server::node_kinds::nodes::ledger::LedgerFactory;
use pii_server::node_kinds::nodes::plan::PlanFactory;
use pii_store::queue::QueueStore;

const FINDING: &str = "The widget renders blank when the list is empty";

/// A spec in the house shape: `## Files` with one backticked path per bullet, `## Exit Criteria` with one command each.
const HOUSE_SPEC: &str = "# Spec: the widget renders when empty\n\n## Requirements\n\n**R1.** The widget renders.\n\n\
## Files\n\n- `crates/pii-server/src/widget.rs`\n\n## Exit Criteria\n\n- [ ] `test -f crates/pii-server/src/widget.rs`\n";

/// A reply with no headings: prose the planner might return when it did not follow the shape.
const PROSE_REPLY: &str = "We should fix the widget so it renders. Nothing more to add.";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A context granting `fs_write` over exactly `root/specs`, which is the only place a plan may write.
fn ctx_over(root: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::FS_WRITE | CapKind::LEDGER;
    caps.fs_write = PathScope::new(vec![dunce::canonicalize(root.join("specs")).unwrap()]);
    Ctx::root(caps, Budget::unlimited())
}

/// A context granting `fs_read` over exactly `root`, which is all `enqueue` needs.
fn read_ctx(root: &Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::FS_READ | CapKind::LEDGER;
    caps.fs_read = PathScope::new(vec![dunce::canonicalize(root).unwrap()]);
    Ctx::root(caps, Budget::unlimited())
}

/// A scratch repository root with a `specs/` directory that holds one seed spec, so the next number is 8.
fn scratch_repo() -> tempfile::TempDir {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "plan-spec-").unwrap();
    std::fs::create_dir_all(dir.path().join("specs")).unwrap();
    std::fs::write(dir.path().join("specs/7-seed.md"), "# Seed\n").unwrap();
    dir
}

/// A stub planner that answers each call with the next reply in `replies`, from a fixture file in `dir`.
fn stub_planner(dir: &Path, replies: &[&str]) -> Arc<pii_model::ScriptedProvider> {
    let turns: Vec<serde_json::Value> = replies
        .iter()
        .map(|text| serde_json::json!({ "text": text }))
        .collect();
    let path = dir.join("planner-fixture.json");
    std::fs::write(&path, serde_json::json!({ "turns": turns }).to_string()).unwrap();
    Arc::new(pii_model::ScriptedProvider::from_fixture(&path).unwrap())
}

/// Runs `plan` over `finding` in `root`, answering `(port, value)`.
fn plan(
    root: &Path,
    provider: Arc<pii_model::ScriptedProvider>,
    finding: &str,
) -> (String, serde_json::Value) {
    let cfg = serde_json::json!({
        "finding": finding,
        "workdir": root.display().to_string(),
        "model": "glm-5.3",
    });
    let node = PlanFactory { provider }.build(&cfg).unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    match node.run(&ctx_over(root), &input).unwrap() {
        Outcome::Next { port, value } => (port.to_string(), value),
        other => panic!("plan yields a port: {other:?}"),
    }
}

/// Runs `enqueue` on the spec `plan` wrote, into `store`, answering `(port, value)`.
fn enqueue(
    root: &Path,
    store: &Arc<QueueStore>,
    spec: &str,
    title: &str,
) -> (String, serde_json::Value) {
    let cfg = serde_json::json!({
        "spec": spec,
        "title": title,
        "workdir": root.display().to_string(),
        "op": "queue.put",
    });
    let node = LedgerFactory {
        queue: Arc::clone(store),
        memory: None,
        heals: None,
        gates: None,
        log: None,
    }
    .build(&cfg)
    .unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    match node.run(&read_ctx(root), &input).unwrap() {
        Outcome::Next { port, value } => (port.to_string(), value),
        other => panic!("enqueue yields a port: {other:?}"),
    }
}

/// The `specs/` entries whose name starts with a number, sorted: the seed and every spec a run wrote.
fn spec_files(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("specs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Writes the one plan row the lane reads for `id`, from the queue item that `enqueue` admitted, into `dir`.
fn write_plan_row(dir: &Path, store: &QueueStore, id: &str) -> PathBuf {
    let item = store.get_item(id).unwrap().expect("the row was enqueued");
    let spec = item.spec.trim_start_matches("specs/");
    let line = format!(
        "{}\t{spec}\t-\t{}\t{}\t{}\t-\n",
        item.id,
        item.title,
        item.criteria.join(";"),
        item.touches.join(";")
    );
    let plan = dir.join("plan.tsv");
    std::fs::write(&plan, line).unwrap();
    plan
}

/// Runs one of the lane's validators (`row-verify.py` or `row-paths.py`) as `check <id>`, over `plan` and `specs`.
fn validator(script: &str, id: &str, plan: &Path, specs: &Path, state: &Path) -> i32 {
    let out = Command::new("python3")
        .arg(repo().join("scripts/autonomy").join(script))
        .args(["check", id])
        .env("PII_PLAN", plan)
        .env("PII_SPECS", specs)
        .env("PII_STATE", state)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.trim().is_empty(), "{script} printed nothing");
    out.status.code().unwrap_or(-1)
}

#[test]
fn a_case_becomes_a_spec_the_validators_accept() {
    let root = scratch_repo();
    let provider = stub_planner(root.path(), &[HOUSE_SPEC]);
    let (port, value) = plan(root.path(), provider, FINDING);
    assert_eq!(port, "ok", "{value}");
    let written = value["spec"].as_str().unwrap().to_owned();
    assert_eq!(value["number"], 8, "the next free number after the seed");
    assert!(written.starts_with("specs/8-"), "{written}");
    assert_eq!(
        spec_files(root.path()).len(),
        2,
        "one new file: {:?}",
        spec_files(root.path())
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join(&written)).unwrap(),
        HOUSE_SPEC,
        "the file holds the planner's reply, unedited"
    );

    let store = Arc::new(QueueStore::open_in_memory().unwrap());
    let (port, value) = enqueue(root.path(), &store, &written, FINDING);
    assert_eq!(port, "ok", "{value}");
    let id = value["id"].as_str().unwrap().to_owned();

    let plan_file = write_plan_row(root.path(), &store, &id);
    let specs = root.path().join("specs");
    let state = root.path().join("state.json");
    assert_eq!(
        validator("row-verify.py", &id, &plan_file, &specs, &state),
        0
    );
    assert_eq!(
        validator("row-paths.py", &id, &plan_file, &specs, &state),
        0
    );
}

#[test]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
fn CONTROL_two_runs_do_not_overwrite() {
    let root = scratch_repo();
    let provider = stub_planner(
        root.path(),
        &[HOUSE_SPEC, "# Second\n\nA different reply.\n"],
    );
    let (port, first) = plan(root.path(), Arc::clone(&provider), FINDING);
    assert_eq!(port, "ok", "{first}");
    let first_path = root.path().join(first["spec"].as_str().unwrap());
    let before = std::fs::read(&first_path).unwrap();

    let (port, second) = plan(root.path(), provider, FINDING);
    assert_eq!(port, "ok", "{second}");
    assert_eq!(second["number"], 9, "the second run takes the next number");
    assert_eq!(
        std::fs::read(&first_path).unwrap(),
        before,
        "the first spec is byte-identical after the second run"
    );
    assert_eq!(
        spec_files(root.path()).len(),
        3,
        "{:?}",
        spec_files(root.path())
    );
}

#[test]
fn a_malformed_spec_is_refused_not_enqueued() {
    let root = scratch_repo();
    let provider = stub_planner(root.path(), &[PROSE_REPLY]);
    let (port, value) = plan(root.path(), provider, FINDING);
    assert_eq!(port, "ok", "{value}");
    let written = value["spec"].as_str().unwrap().to_owned();

    let store = Arc::new(QueueStore::open_in_memory().unwrap());
    let (port, value) = enqueue(root.path(), &store, &written, FINDING);
    assert_eq!(port, "err", "{value}");
    let reason = value["reason"].as_str().unwrap();
    assert!(
        reason.contains("## Files"),
        "the reason names the missing heading: {reason}"
    );
    assert!(
        store.list_items().unwrap().is_empty(),
        "a refused spec leaves the queue unchanged"
    );
}

/// Spec 392 R3: an `agent` drafts and `ledger` op `spec.write` writes: the next free number, the fence stripped, and
/// nothing written without a draft.
#[test]
fn the_write_half_of_a_planner_is_a_ledger_op() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join("specs")).unwrap();
    std::fs::write(root.join("specs/7-older.md"), "older\n").unwrap();
    let factory = LedgerFactory {
        queue: Arc::new(QueueStore::open_in_memory().unwrap()),
        memory: None,
        heals: None,
        gates: None,
        log: None,
    };
    let node = factory
        .build(&serde_json::json!({
            "op": "spec.write",
            "finding": "$.input.finding",
            "workdir": "$.input.workdir",
            "text": "$.payload.text",
        }))
        .unwrap();
    let input = |text: &str| NodeInput {
        input: serde_json::json!({ "finding": FINDING, "workdir": root }),
        payload: serde_json::json!({ "text": text }),
        nodes: serde_json::json!({}),
    };
    let ctx = ctx_over(root);

    let refused = node.run(&ctx, &input("  ")).unwrap();
    assert!(matches!(&refused, Outcome::Next { port, .. } if port.as_str() == "err"));

    let fenced = format!("```markdown\n{HOUSE_SPEC}```\n");
    let Outcome::Next { port, value } = node.run(&ctx, &input(&fenced)).unwrap() else {
        panic!("spec.write took no port");
    };
    assert_eq!(port.as_str(), "ok");
    let spec = value.get("spec").and_then(|s| s.as_str()).unwrap();
    assert!(spec.starts_with("specs/8-"), "{spec}");
    let written = std::fs::read_to_string(root.join(spec)).unwrap();
    assert!(written.starts_with("# Spec:"), "{written}");
    assert!(!written.contains("```"), "{written}");
}
