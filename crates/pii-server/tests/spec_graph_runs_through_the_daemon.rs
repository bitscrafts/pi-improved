//! Spec 284: `graphs/spec.toml` runs through the daemon's engine, `Engine::start_run`, which is the path `run start` reaches.
//! The tracked graph and its called `docs` graph run with the daemon's own budget, registry, capability grant and journal;
//! the agents are a fake `pi` and `claude` on `PATH`, so nothing spends. Every assertion reads the engine's journal, except
//! the agent argv, which the fake `claude` records.
//! The missing-`docs_brief` control is LATE: its `require` runs after the host's `code` coder, not at run start.
//! Spec 288 moves that check to run start.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod spec_graph_daemon;

use pii_abi::RunStatus;
use spec_graph_daemon::{
    Case, DOCS_BREAK, DOCS_OK, graphs_copy, in_child, observed, settle, started, tracked_graphs,
    tracked_limit_micro, usd_flag,
};

const CRITERION: &str = "test -f built.txt";

/// The journal's `graph.call` observation for `docs`: its head node and its fields.
fn docs_call(journal: &[(u64, pii_abi::RunEvent)]) -> (String, serde_json::Value) {
    let calls: Vec<_> = observed(journal, "graph.call")
        .into_iter()
        .filter(|(_, f)| f["graph"] == "docs")
        .collect();
    assert_eq!(calls.len(), 1, "exactly one graph.call for docs: {calls:?}");
    calls.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_passing_run_documents_through_the_daemon() {
    let Some(bin) = in_child("a_passing_run_documents_through_the_daemon", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );

    // The nodes ran in the spec's order, with the called graph's head and coder between verify and reverify. The coder
    // ids are read from the journal, and the call's own plumbing nodes (its input and its need check) are left out.
    let (head, _) = docs_call(&journal);
    let coders: Vec<String> = observed(&journal, "coder.spawned")
        .into_iter()
        .map(|(node, _)| node)
        .collect();
    assert_eq!(coders.len(), 2, "both coders spawned: {coders:?}");
    let ran: Vec<String> = started(&journal)
        .into_iter()
        .filter(|n| {
            *n == head || *n == "verify" || *n == "reverify" || *n == "done" || coders.contains(n)
        })
        .collect();
    // Spec 385 moved the implementer (and its `verify`) inside the methodology the driver picks, so the
    // coder ids are namespaced under `method-judge` and only the HOST's tail is seen here: the docs step,
    // the re-judge, the end.
    let expected = vec![
        coders[0].clone(),
        head,
        coders[1].clone(),
        "reverify".to_owned(),
        "done".to_owned(),
    ];
    assert_eq!(
        ran, expected,
        "the docs step runs between verify and reverify"
    );

    // The journal pins the sub-graph by the full digest of graphs/docs.toml as it is on disk now.
    let (_, fields) = docs_call(&journal);
    let on_disk = std::fs::read(tracked_graphs().join("docs.toml")).unwrap();
    assert_eq!(
        fields["sha256"],
        pii_graph::fragment::sha256_hex(&on_disk),
        "{fields}"
    );
}

#[tokio::test]
async fn the_two_coders_have_two_sessions_and_are_briefed_apart() {
    let Some(bin) = in_child(
        "the_two_coders_have_two_sessions_and_are_briefed_apart",
        DOCS_OK,
    ) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );

    let spawned = observed(&journal, "coder.spawned");
    assert_eq!(spawned.len(), 2, "both coders spawned: {spawned:?}");
    assert_ne!(
        spawned[0].1["session"], spawned[1].1["session"],
        "two coders, two sessions: {spawned:?}"
    );

    let argv = case.agent_argv();
    assert_eq!(argv.len(), 2, "two agent calls: {argv:?}");
    assert!(
        argv[0].contains("IMPL-BRIEF") && !argv[0].contains("DOCS-BRIEF"),
        "{}",
        argv[0]
    );
    assert!(
        argv[1].contains("DOCS-BRIEF") && !argv[1].contains("IMPL-BRIEF"),
        "{}",
        argv[1]
    );
}

#[tokio::test]
async fn the_budget_is_shared_through_the_call() {
    let Some(bin) = in_child("the_budget_is_shared_through_the_call", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );

    // The `[limits]` ceiling beats the input's $9.00 for the first coder; the docs coder inside the called graph gets
    // what the first one left after charging $0.40, which proves the call shares the host's budget. Both numbers come
    // from the tracked graph, so a change of policy moves them and not this test.
    let limit = tracked_limit_micro();
    let argv = case.agent_argv();
    assert_eq!(argv.len(), 2, "two agent calls: {argv:?}");
    let first = format!("--max-budget-usd {} ", usd_flag(limit));
    let second = format!("--max-budget-usd {} ", usd_flag(limit - 400_000));
    assert!(argv[0].contains(&first), "{}", argv[0]);
    assert!(argv[1].contains(&second), "{}", argv[1]);
}

#[tokio::test]
async fn CONTROL_a_docs_step_that_breaks_the_criteria_fails_the_run() {
    let Some(bin) = in_child(
        "CONTROL_a_docs_step_that_breaks_the_criteria_fails_the_run",
        DOCS_BREAK,
    ) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Failed),
        "{status:?}: {journal:?}"
    );
    let order = started(&journal);
    assert!(
        order.contains(&"reverify".to_owned()),
        "reverify ran: {order:?}"
    );
    assert!(
        !order.contains(&"done".to_owned()),
        "done never runs: {order:?}"
    );
}

#[tokio::test]
async fn CONTROL_a_missing_docs_brief_refuses_the_called_graph() {
    let Some(bin) = in_child(
        "CONTROL_a_missing_docs_brief_refuses_the_called_graph",
        DOCS_OK,
    ) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = case.engine(&tracked_graphs());
    // Spec 288: the call's need is checked at start, so the refusal is the start's and no run exists for it.
    let Err(err) = engine.start_run("spec", &case.input(false)) else {
        panic!("a missing docs need refuses the run at start");
    };
    assert!(
        err.to_string()
            .contains("graph.call.need: call 'docs' graph 'docs' needs 'brief'"),
        "the call refuses on its need, naming call, graph and key: {err}"
    );
    // Refused at start, nothing spawned: neither the host's `code` coder nor the called graph's runs.
    assert!(
        case.agent_argv().is_empty(),
        "nothing started: {:?}",
        case.agent_argv()
    );
}

#[tokio::test]
async fn CONTROL_a_changed_docs_graph_refuses_at_start() {
    // No agent runs here: the refusal happens at start, so the bin directory is empty.
    let bin =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "daemon-pin-bin-").unwrap();
    let case = Case::new(bin.path(), CRITERION);
    let copy = graphs_copy(case.dir.path(), None);
    let docs_path = copy.join("docs.toml");
    let docs = std::fs::read_to_string(&docs_path).unwrap();
    let changed = docs.replacen("1200000", "1200001", 1);
    assert_ne!(docs, changed, "the fixture changes one byte");
    std::fs::write(&docs_path, &changed).unwrap();
    let engine = case.engine(&copy);
    let Err(err) = engine.start_run("spec", &case.input(true)) else {
        panic!("a changed sub-graph is refused at start");
    };
    let msg = err.to_string();
    let pinned = pii_graph::fragment::sha256_hex(docs.as_bytes());
    let actual = pii_graph::fragment::sha256_hex(changed.as_bytes());
    assert!(msg.contains("fragment.pin.mismatch"), "{msg}");
    assert!(
        msg.contains(&pinned) && msg.contains(&actual),
        "both digests named: {msg}"
    );
    assert!(
        case.agent_argv().is_empty(),
        "nothing started: {:?}",
        case.agent_argv()
    );
}

#[tokio::test]
async fn CONTROL_an_exhausted_budget_spawns_no_docs_coder() {
    let Some(bin) = in_child("CONTROL_an_exhausted_budget_spawns_no_docs_coder", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    // $0.10 of budget: the first coder's $0.40 exhausts it, and the engine stops the run before the call.
    let copy = graphs_copy(case.dir.path(), Some(100_000));
    let engine = case.engine(&copy);
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Failed),
        "{status:?}: {journal:?}"
    );
    assert_eq!(
        observed(&journal, "coder.spawned").len(),
        1,
        "only the code coder spawns"
    );
    assert_eq!(
        case.agent_argv().len(),
        1,
        "no docs agent is called: {:?}",
        case.agent_argv()
    );
    assert!(
        !started(&journal).contains(&"reverify".to_owned()),
        "reverify never runs"
    );
}
