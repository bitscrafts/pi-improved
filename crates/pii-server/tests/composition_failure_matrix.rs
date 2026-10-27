//! Spec 285: a composed run survives cancel, restart, concurrency and reuse, through `Engine::start_run`.
//!
//! Each case hosts a sub-graph call (`docs`, or a graph the case writes) whose coder is a fake `claude`: it hangs
//! (`DOCS_HANG`, recording its shell pid in `pids`) or finishes (`DOCS_OK`). Nothing spends. The graphs and helpers
//! live in `spec_graph_daemon/mod.rs`.
//!
//! MEASURED on this tree (the assertions pin each one):
//! - M1: a kill reaches a coder inside an inlined call (`docs__docs`), so M1 passes.
//! - M2: one process cannot reopen its own live store, and a run thread holds the engine, so a restart is a copy of
//!   the store file taken while the agent hangs. Boot recovery closes the run `Failed` ("interrupted by daemon
//!   restart") and keeps its `graph.call`, but does NOT kill the hung agent: `coder.spawned` reaches the journal only
//!   when its node returns (`pii-graph/src/sched/dispatch.rs`), and the reaper finds pids only there. M2 and M7 fail
//!   on the surviving agent. A live coder is seen through its node's start and the pid file.
//! - M3: the same, so the live call coders' sessions are read once their node ends; the code coders' are read now.
//! - M6: the loader accepts a `call` in a loop body; the body's coder runs on each pass until `loop_budget` ends the
//!   run `Failed`; the journal holds one `graph.call` for the whole loop.
//! - M7: resume exists (`spawn_resume`) and refuses a run that already finished, naming it.
//! - A cancel closes a run `Failed` with `cancelled: <reason>`: the spec's `Cancelled` terminal is `Failed` here.

#![allow(
    non_snake_case,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod spec_graph_daemon;

use std::sync::Arc;

use pii_abi::RunStatus;
use pii_server::engine::orphans::recover_at_boot;
use pii_server::engine::runs::spawn_resume;
use pii_server::engine::runs_cancel::{KILL_WAIT, cancel_run, cancel_run_with};
use spec_graph_daemon::{
    Case, DOCS_HANG, DOCS_OK, LOOPED, Sleepers, argv_of, call_coder_started, docs_calls,
    docs_digest, graphs_copy, in_child, is_call_coder, needy_engine, observed, reason, restarted,
    restarted_when_durable, settle, started, tracked_graphs, twice_engine, wait_dead, wait_pids,
};

const CRITERION: &str = "test -f built.txt";

#[tokio::test]
async fn m1_cancel_kill_inside_a_call() {
    let Some(bin) = in_child("m1_cancel_kill_inside_a_call", DOCS_HANG) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    call_coder_started(&engine, &run).await;
    let pids = wait_pids(&sleepers, 1).await;

    cancel_run_with(&engine, &run, "operator", true, KILL_WAIT).expect("a kill stops the call");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Failed),
        "{status:?}: {journal:?}"
    );
    assert!(reason(&journal).contains("cancelled:"), "{journal:?}");
    for pid in pids {
        assert!(wait_dead(pid).await, "agent {pid} outlived the kill");
    }
    assert_eq!(
        docs_calls(&journal).len(),
        1,
        "exactly one graph.call: {journal:?}"
    );
    assert!(
        !started(&journal).contains(&"reverify".to_owned()),
        "reverify never starts"
    );
}

#[tokio::test]
async fn m1_control_cancel_without_kill_is_refused_inside_a_call() {
    let Some(bin) = in_child(
        "m1_control_cancel_without_kill_is_refused_inside_a_call",
        DOCS_HANG,
    ) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    call_coder_started(&engine, &run).await;
    let pids = wait_pids(&sleepers, 1).await;

    let err = cancel_run(&engine, &run, "operator").expect_err("a running node refuses a cancel");
    assert!(err.to_string().contains("cancel refused"), "{err}");
    cancel_run_with(&engine, &run, "operator", true, KILL_WAIT).expect("a kill stops the call");
    settle(&engine, &run).await;
    for pid in pids {
        assert!(wait_dead(pid).await, "agent {pid} outlived the kill");
    }
}

#[tokio::test]
async fn m2_restart_inside_a_call() {
    let Some(bin) = in_child("m2_restart_inside_a_call", DOCS_HANG) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    call_coder_started(&engine, &run).await;
    let pids = wait_pids(&sleepers, 1).await;

    let engine2 = restarted_when_durable(&case, &tracked_graphs(), &run, 2).await;
    recover_at_boot(&engine2);
    let (status, journal) = settle(&engine2, &run).await;
    assert!(
        matches!(status, RunStatus::Failed),
        "closed, not left Running: {status:?}"
    );
    assert!(
        reason(&journal).contains("interrupted by daemon restart"),
        "{journal:?}"
    );
    let calls = docs_calls(&journal);
    assert!(
        calls.len() == 1 && calls[0].1["sha256"] == docs_digest(),
        "the graph.call survives with its digest: {calls:?}"
    );
    let spawns = observed(&journal, "coder.spawned").len();
    for pid in pids {
        let gone = wait_dead(pid).await;
        assert!(
            gone,
            "agent {pid} survives the restart; the journal holds {spawns} coder.spawned"
        );
    }
}

#[tokio::test]
async fn m2_control_a_finished_run_is_untouched_by_a_restart() {
    let Some(bin) = in_child(
        "m2_control_a_finished_run_is_untouched_by_a_restart",
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

    let engine2 = restarted(&case, &tracked_graphs());
    recover_at_boot(&engine2);
    let header = engine2.store.get_run(&run).unwrap().unwrap();
    assert!(
        matches!(header.status, RunStatus::Succeeded),
        "{:?}",
        header.status
    );
    assert_eq!(
        engine2.store.read_journal(&run).unwrap().len(),
        journal.len()
    );
}

#[tokio::test]
async fn m3_two_concurrent_runs_of_one_graph() {
    let Some(bin) = in_child("m3_two_concurrent_runs_of_one_graph", DOCS_HANG) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let input = |brief: &str| {
        let mut v = case.input(true);
        v["brief"] = serde_json::json!(brief);
        v
    };
    let run_a = engine
        .start_run("spec", &input("IMPL-A"))
        .expect("run a starts");
    let run_b = engine
        .start_run("spec", &input("IMPL-B"))
        .expect("run b starts");
    assert_ne!(run_a, run_b, "two run ids");
    call_coder_started(&engine, &run_a).await;
    call_coder_started(&engine, &run_b).await;
    wait_pids(&sleepers, 2).await;

    // The code coders finished before the call, so their spawns are journalled; the live call coders are not yet.
    let (journal_a, journal_b) = (
        engine.store.read_journal(&run_a).unwrap(),
        engine.store.read_journal(&run_b).unwrap(),
    );
    let code = |journal: &[(u64, pii_abi::RunEvent)]| {
        observed(journal, "coder.spawned")
            .into_iter()
            .find(|(node, _)| !is_call_coder(node))
            .expect("the code coder")
            .1["session"]
            .clone()
    };
    let (session_a, session_b) = (code(&journal_a), code(&journal_b));
    assert_ne!(session_a, session_b, "two runs, two sessions");
    assert_eq!(docs_calls(&journal_a).len(), 1, "run a's graph.call");
    assert_eq!(docs_calls(&journal_b).len(), 1, "run b's graph.call");

    // Each run's own coder carries its own brief, and never the other's.
    let argv_a = argv_of(&case, session_a.as_str().unwrap());
    let argv_b = argv_of(&case, session_b.as_str().unwrap());
    assert!(
        argv_a.contains("IMPL-A") && !argv_a.contains("IMPL-B"),
        "{argv_a}"
    );
    assert!(
        argv_b.contains("IMPL-B") && !argv_b.contains("IMPL-A"),
        "{argv_b}"
    );
}

#[tokio::test]
async fn m3_control_one_cancel_leaves_the_other_run_alone() {
    let Some(bin) = in_child(
        "m3_control_one_cancel_leaves_the_other_run_alone",
        DOCS_HANG,
    ) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let run_a = engine
        .start_run("spec", &case.input(true))
        .expect("run a starts");
    let run_b = engine
        .start_run("spec", &case.input(true))
        .expect("run b starts");
    call_coder_started(&engine, &run_a).await;
    call_coder_started(&engine, &run_b).await;
    wait_pids(&sleepers, 2).await;

    cancel_run_with(&engine, &run_a, "operator", true, KILL_WAIT).expect("run a is killed");
    let (status_a, _) = settle(&engine, &run_a).await;
    assert!(matches!(status_a, RunStatus::Failed), "{status_a:?}");
    let status_b = engine.store.get_run(&run_b).unwrap().unwrap().status;
    assert!(
        matches!(status_b, RunStatus::Running) && engine.is_in_flight(run_b.as_str()),
        "run b runs on: {status_b:?}"
    );

    cancel_run_with(&engine, &run_b, "operator", true, KILL_WAIT).expect("run b is killed");
    settle(&engine, &run_b).await;
}

#[tokio::test]
async fn m4_one_graph_called_twice() {
    let Some(bin) = in_child("m4_one_graph_called_twice", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = twice_engine(&case);
    let run = engine
        .start_run("twice", &case.input(true))
        .expect("the host starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );

    let calls = docs_calls(&journal);
    assert_eq!(calls.len(), 2, "two graph.call observations: {calls:?}");
    assert_ne!(calls[0].0, calls[1].0, "different call ids");
    assert_eq!(calls[0].1["sha256"], calls[1].1["sha256"], "one digest");
    let sessions: Vec<_> = observed(&journal, "coder.spawned")
        .into_iter()
        .filter(|(node, _)| is_call_coder(node))
        .map(|(_, f)| f["session"].clone())
        .collect();
    assert_eq!(sessions.len(), 2, "both sub-runs spawned: {journal:?}");
    assert_ne!(sessions[0], sessions[1], "two sub-runs, two sessions");
}

#[tokio::test]
async fn m4_control_the_host_journal_node_ids_are_unique() {
    let Some(bin) = in_child("m4_control_the_host_journal_node_ids_are_unique", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let engine = twice_engine(&case);
    let run = engine
        .start_run("twice", &case.input(true))
        .expect("the host starts");
    let (_, journal) = settle(&engine, &run).await;
    let ids = started(&journal);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "no node started twice: {ids:?}");
}

#[tokio::test]
async fn m5_budget_exhausted_inside_a_called_graph() {
    let Some(bin) = in_child("m5_budget_exhausted_inside_a_called_graph", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    // $0.10 of budget: the first coder's $0.40 exhausts it before the call.
    let engine = case.engine(&graphs_copy(case.dir.path(), Some(100_000)));
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
        "only the host's coder spawns"
    );
    assert_eq!(case.agent_argv().len(), 1, "the called coder never starts");
    assert!(
        reason(&journal).contains("budget"),
        "the reason is journalled: {journal:?}"
    );
}

#[tokio::test]
async fn m5_control_the_default_budget_completes_the_call() {
    let Some(bin) = in_child("m5_control_the_default_budget_completes_the_call", DOCS_OK) else {
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
    assert_eq!(
        observed(&journal, "coder.spawned").len(),
        2,
        "both coders spawn"
    );
}

#[tokio::test]
async fn m6_a_call_inside_a_loop_body() {
    let Some(bin) = in_child("m6_a_call_inside_a_loop_body", DOCS_OK) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let graphs = graphs_copy(case.dir.path(), None);
    std::fs::write(
        graphs.join("looped.toml"),
        LOOPED.replace("SHA", &docs_digest()),
    )
    .unwrap();
    let engine = case.engine(&graphs);
    // MEASURED: the loader accepts a call inside a loop body, so the run starts and its loop ends on its budget.
    let run = engine
        .start_run("looped", &case.input(true))
        .expect("the loader accepts a call inside a loop");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Failed),
        "{status:?}: {journal:?}"
    );
    assert!(
        reason(&journal).contains("loop_budget"),
        "the loop ends on its budget: {journal:?}"
    );
    let passes = observed(&journal, "coder.spawned")
        .iter()
        .filter(|(node, _)| is_call_coder(node))
        .count();
    assert!(
        passes >= 2,
        "the call's coder runs on each pass: {journal:?}"
    );
}

#[tokio::test]
async fn m7_resume_of_a_run_interrupted_inside_a_call_is_refused() {
    let Some(bin) = in_child(
        "m7_resume_of_a_run_interrupted_inside_a_call_is_refused",
        DOCS_HANG,
    ) else {
        return;
    };
    let case = Case::new(&bin, CRITERION);
    let sleepers = Sleepers::new(&case.bin);
    let engine = case.engine(&tracked_graphs());
    let run = engine
        .start_run("spec", &case.input(true))
        .expect("the spec graph starts");
    call_coder_started(&engine, &run).await;
    let pids = wait_pids(&sleepers, 1).await;

    let engine2 = restarted_when_durable(&case, &tracked_graphs(), &run, 2).await;
    recover_at_boot(&engine2);
    settle(&engine2, &run).await;
    // MEASURED: the engine has a resume verb, and it refuses a run that already finished, naming it.
    let err = spawn_resume(Arc::clone(&engine2), run.clone(), None)
        .expect_err("a finished run is never resumed");
    let msg = err.to_string();
    assert!(
        msg.contains(run.as_str()) && msg.contains("refused"),
        "{msg}"
    );
    for pid in pids {
        assert!(wait_dead(pid).await, "agent {pid} survives the restart");
    }
}

#[tokio::test]
async fn m8_needs_refused_through_the_engine() {
    // Spec 288: the call's declared `needs` that the run input cannot supply refuse the run at START, before any run exists
    // (spec 267's run-time node, which refused after the host's own nodes had run, is still the second line of defence).
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "m8-bin-").unwrap();
    let case = Case::new(bin.path(), CRITERION);
    let engine = needy_engine(&case);
    let err = engine
        .start_run("needy-host", &serde_json::json!({ "spec": "hello" }))
        .expect_err("a missing needs key refuses at start");
    let text = err.to_string();
    assert!(
        text.contains("graph.call.need")
            && text.contains("call 'need'")
            && text.contains("graph 'needy'")
            && text.contains("needs 'x'"),
        "the refusal names the call, the graph and the key: {text}"
    );
    for status in [
        RunStatus::Running,
        RunStatus::Suspended,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Refused,
    ] {
        let runs = engine.store.runs_by_status(status).unwrap();
        assert!(runs.is_empty(), "no run is created: {status:?} {runs:?}");
    }
}

#[tokio::test]
async fn m8_control_with_the_need_present_the_call_runs() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "m8-bin-").unwrap();
    let case = Case::new(bin.path(), CRITERION);
    let engine = needy_engine(&case);
    let input = serde_json::json!({ "spec": "hello", "x": "present" });
    let run = engine
        .start_run("needy-host", &input)
        .expect("the host starts");
    let (status, journal) = settle(&engine, &run).await;
    assert!(
        matches!(status, RunStatus::Succeeded),
        "{status:?}: {journal:?}"
    );
}
