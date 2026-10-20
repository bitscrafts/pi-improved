//! Spec 212: a fragment is inlined at a named surface, namespaced, stamped with
//! its origin, ordered deterministically, shares the host's budget, and a
//! blocking hook refuses the run by name with no host node executed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::{RunEvent, Terminal};
use pii_core::{Budget, CapSet, Ctx, Observer, RecordingObserver, TraceCtx};
use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;
use pii_graph::run;
use pii_store::Store;

const GUARD: &str = r#"
name = "podman-guard"

[fragment]
name = "podman-guard"
inputs = ["$.input.runtime"]
outputs = ["ok", "refuse"]
may_block = true

[[node]]
id = "check"
kind = "route"
on = "$.input.runtime"
rules = [{ eq = "podman", port = "refuse" }, { port = "ok" }]

[[node]]
id = "refuse"
kind = "refuse"
reason = "podman is refused by the guard"

[[edge]]
from = "check"
port = "refuse"
to = "refuse"
kind = "data"
"#;

const HOST: &str = r#"
name = "host"
entry = "build"

[limits]
max_steps = 7

[[attach]]
id = "guard"
fragment = "podman-guard"
at = "run.start"
pin = "PIN"

[[node]]
id = "build"
kind = "transform"
value = "built"

[[node]]
id = "deploy"
kind = "transform"
value = "deployed"

[[edge]]
from = "build"
port = "ok"
to = "deploy"
kind = "data"
"#;

/// Writes `graphs/fragments/<name>.toml` files under a fresh temp dir and
/// returns the temp dir (kept alive by the caller) and the fragments dir.
fn fragments(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("tempdir");
    let dir = root.path().join("fragments");
    std::fs::create_dir_all(&dir).expect("mkdir fragments");
    for (name, src) in files {
        std::fs::write(dir.join(format!("{name}.toml")), src).expect("write fragment");
    }
    (root, dir)
}

/// The host with its `PIN` placeholder replaced by the guard's real hash.
fn host_with_pin(host: &str, guard: &str) -> String {
    host.replace("PIN", &fragment::sha256_hex(guard.as_bytes()))
}

fn weave_ok(host: &str, frags: &[(&str, &str)]) -> (tempfile::TempDir, Woven) {
    let (root, dir) = fragments(frags);
    let woven = fragment::weave(host, &dir, &NodeRegistry::builtin()).expect("weave");
    (root, woven)
}

fn weave_err(host: &str, frags: &[(&str, &str)]) -> String {
    let (_root, dir) = fragments(frags);
    fragment::weave(host, &dir, &NodeRegistry::builtin())
        .expect_err("weave must refuse")
        .to_string()
}

/// Runs the woven graph once and returns its terminal and journal.
fn run_woven(woven: &Woven, input: serde_json::Value) -> (Terminal, Vec<(u64, RunEvent)>) {
    let root = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&root.path().join("db.redb")).expect("store open");
    let run_id = store.next_run_id().expect("run id");
    let obs = Arc::new(RecordingObserver::new());
    let ctx = Ctx::with_observer(
        CapSet::empty(),
        Budget::unlimited(),
        Arc::clone(&obs) as Arc<dyn Observer>,
        TraceCtx {
            run: run_id.clone(),
            span: 0,
        },
    );
    let reg = NodeRegistry::builtin();
    let terminal =
        run::run_with_validate(&store, &woven.spec, &reg, &ctx, &run_id, input, true).expect("run");
    let journal = store.read_journal(&run_id).expect("journal");
    (terminal, journal)
}

/// Spec 212 contract: `fragment_inlines_at_run_start`.
#[test]
fn fragment_inlines_at_run_start() {
    let host = host_with_pin(HOST, GUARD);
    let (_root, woven) = weave_ok(&host, &[("podman-guard", GUARD)]);

    assert_eq!(woven.spec.entry.as_str(), "guard.check");
    let ids: Vec<&str> = woven.spec.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"guard.check"), "namespaced node: {ids:?}");
    assert!(ids.contains(&"guard.refuse"), "namespaced node: {ids:?}");

    let check = woven
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "guard.check")
        .expect("guard.check");
    assert_eq!(
        check.cfg["woven-from"],
        serde_json::json!("podman-guard#check")
    );
    assert_eq!(check.cfg["hook"], serde_json::json!("guard"));

    // The fragment's ok exit feeds the host's original entry.
    assert!(
        woven
            .spec
            .edges
            .iter()
            .any(|e| e.from.as_str() == "guard.check"
                && e.port.as_str() == "ok"
                && e.to.as_str() == "build"),
        "ok exit must reach the host entry"
    );
    assert_eq!(woven.sequence.len(), 2);
    assert!(
        woven
            .sequence
            .iter()
            .all(|i| i.origin.starts_with("podman-guard#") && i.surface == "run.start")
    );
}

/// Spec 212 contract: `block_only_where_allowed`.
#[test]
fn block_only_where_allowed() {
    let advisory = GUARD.replace("may_block = true", "may_block = false");
    let host = host_with_pin(HOST, &advisory).replace(
        "at = \"run.start\"",
        "at = \"node.before\"\non = [\"deploy\"]",
    );
    let msg = weave_err(&host, &[("podman-guard", &advisory)]);
    assert!(msg.contains("fragment.block.invalid"), "got: {msg}");
}

/// Spec 212 contract: `budget_is_shared` — a three-node fragment adds its
/// nodes to the host, and the host's budget is the only budget.
#[test]
fn budget_is_shared() {
    let three = GUARD.replace(
        "[[edge]]",
        "[[node]]\nid = \"audit\"\nkind = \"transform\"\nvalue = \"seen\"\n\n[[node]]\nid = \"note\"\nkind = \"transform\"\nvalue = \"noted\"\n\n[[edge]]",
    );
    let host = host_with_pin(HOST, &three);
    let (_root, woven) = weave_ok(&host, &[("podman-guard", &three)]);

    assert_eq!(woven.spec.limits.max_steps, 7, "host budget is unchanged");
    // Host: build, deploy. Fragment: check, refuse, audit, note.
    assert_eq!(woven.spec.nodes.len(), 6);
    assert_eq!(woven.sequence.len(), 4);

    // A fragment that declares its own budget is refused.
    let greedy = three.replacen("[[node]]", "[limits]\nmax_steps = 99\n\n[[node]]", 1);
    let msg = weave_err(&host_with_pin(HOST, &greedy), &[("podman-guard", &greedy)]);
    assert!(msg.contains("fragment.autonomy.forbidden"), "got: {msg}");
}

/// Spec 212 contract: `order_is_deterministic` — same surface, sequence is
/// `order` then `id`, identical however the attaches were declared.
#[test]
fn order_is_deterministic() {
    let attach = |id: &str, order: i32| {
        format!(
            "[[attach]]\nid = \"{id}\"\nfragment = \"podman-guard\"\nat = \"run.start\"\norder = {order}\npin = \"{}\"\n",
            fragment::sha256_hex(GUARD.as_bytes())
        )
    };
    let forward = HOST.replace(
        "[[attach]]\nid = \"guard\"\nfragment = \"podman-guard\"\nat = \"run.start\"\npin = \"PIN\"\n",
        &format!("{}{}{}", attach("zeta", 0), attach("alpha", 0), attach("first", -1)),
    );
    let reversed = HOST.replace(
        "[[attach]]\nid = \"guard\"\nfragment = \"podman-guard\"\nat = \"run.start\"\npin = \"PIN\"\n",
        &format!("{}{}{}", attach("first", -1), attach("alpha", 0), attach("zeta", 0)),
    );
    let order_of = |woven: &Woven| {
        let mut seen: Vec<String> = Vec::new();
        for i in &woven.sequence {
            if seen.last() != Some(&i.attach) {
                seen.push(i.attach.clone());
            }
        }
        seen
    };
    let (_a, a) = weave_ok(&forward, &[("podman-guard", GUARD)]);
    let (_b, b) = weave_ok(&reversed, &[("podman-guard", GUARD)]);
    assert_eq!(order_of(&a), ["first", "alpha", "zeta"]);
    assert_eq!(a.sequence, b.sequence, "declaration order must not matter");
}

/// Spec 212 contract: `refusal_names_the_hook` — a blocking hook that refuses
/// fails the run naming the hook, and no host node executes.
#[test]
fn refusal_names_the_hook() {
    let host = host_with_pin(HOST, GUARD);
    let (_root, woven) = weave_ok(&host, &[("podman-guard", GUARD)]);

    let (terminal, journal) = run_woven(&woven, serde_json::json!({ "runtime": "podman" }));
    match terminal {
        Terminal::Failure { error } => {
            assert!(error.contains("run.refused{hook=guard"), "got: {error}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let host_ran = journal.iter().any(|(_, ev)| {
        matches!(ev, RunEvent::NodeFinished { node, .. } if node.as_str() == "build" || node.as_str() == "deploy")
    });
    assert!(!host_ran, "no host node may execute after a refusal");

    // The same graph with no refusing input runs to success.
    let (terminal, _) = run_woven(&woven, serde_json::json!({}));
    assert!(
        matches!(terminal, Terminal::Success { .. }),
        "unrefused run must succeed, got {terminal:?}"
    );
}
