//! The `github-issue` graph end to end: what a delivered issue produces, and what it must never produce.
//!
//! Split from a single 450-line file (spec 423) because the harness needed room for the `plan` model role; the
//! cargo target name is unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

mod harness;

use harness::*;

#[test]
fn the_graph_lints_and_has_no_queue_or_coder_node() {
    let src = graph_src();
    let reg = pii_server::lint_registry(Path::new("."), Path::new("."));
    fragment::load_host_with_graphs(
        &src,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &reg,
    )
    .expect("the github-issue graph lints");

    let forbidden = ["queue.put", "queue.claim", "call", "coder", "spawn", "proc"];
    let present: Vec<String> = kinds(&src)
        .into_iter()
        .filter(|k| forbidden.contains(&k.as_str()))
        .collect();
    assert!(
        present.is_empty(),
        "the draft graph runs no queue, call, coder, spawn or proc node: {present:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delivered_issue_gets_an_ack_a_drafted_spec_and_a_path() {
    let dir = scratch("github-issue-draft-");
    let planner = planner_fixture(dir.path(), &[HOUSE_SPEC]);
    let (d, github) = daemon_in(dir.path(), &planner);

    let (run, trace) = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");

    let sent = github.sent.lock().clone();
    assert_eq!(
        sent.len(),
        2,
        "an acknowledgement, then the drafted path: {sent:?}"
    );
    assert!(sent.iter().all(|(to, _)| to == "7"), "{sent:?}");

    let ws = dir.path().join("ws");
    let new: Vec<String> = spec_names(&ws)
        .into_iter()
        .filter(|n| n != "7-seed.md")
        .collect();
    assert_eq!(new.len(), 1, "one new file under specs/: {new:?}");
    assert!(new[0].starts_with("8-"), "the next free number: {new:?}");
    assert!(
        sent[1].1.contains(&new[0]),
        "the last post names the drafted spec: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drafted_run_ends_with_an_empty_reply_text() {
    let dir = scratch("github-issue-end-");
    let planner = planner_fixture(dir.path(), &[HOUSE_SPEC]);
    let (d, _github) = daemon_in(dir.path(), &planner);

    let (_, trace) = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    assert_eq!(final_output(&trace)["text"], "", "no reply text");
}

#[tokio::test(flavor = "multi_thread")]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_refused_run_also_ends_with_an_empty_reply_text() {
    let dir = scratch("github-issue-end-refused-");
    let planner = planner_fixture(dir.path(), &[]);
    let (d, github) = daemon_in(dir.path(), &planner);

    let (_, trace) = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    assert_eq!(final_output(&trace)["text"], "", "no reply text");
    let sent = github.sent.lock().clone();
    assert_eq!(sent.iter().filter(|(_, t)| t.contains(REFUSAL)).count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_issues_text_reaches_only_the_planner() {
    let graph_nodes = nodes(&graph_src());
    for n in &graph_nodes {
        let id = n.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if id == "plan" {
            continue;
        }
        let cfg = n.to_string();
        assert!(
            !cfg.contains("input.text") && !cfg.contains(INJECTED),
            "node {id} is handed the issue text: {cfg}"
        );
    }
    for id in ["ack", "done", "refused"] {
        let text = graph_nodes
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|n| n.get("text").or_else(|| n.get("template")))
            .and_then(|t| t.as_str())
            .unwrap_or_else(|| panic!("node {id} has neither text nor template"));
        assert!(
            !text.contains("$.input") && !text.contains("${input") && !text.contains("$.payload"),
            "node {id} posts a fixed sentence, not the issue: {text}"
        );
    }

    let dir = scratch("github-issue-inject-");
    let planner = planner_fixture(dir.path(), &[HOUSE_SPEC]);
    let (d, github) = daemon_in(dir.path(), &planner);
    let (run, trace) = run_graph(&d.engine, delivery(7, INJECTED)).await;
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");
    for (_, text) in github.sent.lock().iter() {
        assert!(
            !text.contains("ignore") && !text.contains("run the queue"),
            "a comment carries the issue's text: {text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_planner_failure_posts_the_fixed_refusal_and_writes_no_file() {
    let dir = scratch("github-issue-fail-");
    // No turns: the planner's call fails, as a planner that errors does.
    let planner = planner_fixture(dir.path(), &[]);
    let (d, github) = daemon_in(dir.path(), &planner);

    let _ = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    let sent = github.sent.lock().clone();
    assert_eq!(sent.len(), 2, "one acknowledgement, one refusal: {sent:?}");
    let refusal = &sent[1].1;
    assert!(refusal.contains(REFUSAL), "the fixed refusal: {refusal}");
    assert!(
        !refusal.contains("exhausted"),
        "the refusal carries nothing from the failure: {refusal}"
    );

    let new: Vec<String> = spec_names(&dir.path().join("ws"))
        .into_iter()
        .filter(|n| n != "7-seed.md")
        .collect();
    assert!(new.is_empty(), "no spec is written: {new:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_second_delivery_posts_nothing_twice() {
    let dir = scratch("github-issue-twice-");
    let planner = planner_fixture(dir.path(), &[HOUSE_SPEC, HOUSE_SPEC]);
    let (d, github) = daemon_in(dir.path(), &planner);

    let (first, _) = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    assert_eq!(first["status"], "succeeded", "{first}");
    let after_first = github.sent.lock().len();
    assert_eq!(after_first, 2, "the first delivery posts its two comments");

    // The channel's cursor did not advance, so the same issue arrives again.
    let (second, _) = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    assert_eq!(second["status"], "succeeded", "{second}");
    let sent = github.sent.lock().clone();
    assert_eq!(
        sent.len(),
        after_first,
        "the second delivery posts nothing: {sent:?}"
    );
}

/// Near-miss of the test above: a second issue still gets its comments. Not a guard on the key text: the daemon dedupes on
/// (channel, destination, key), so the destination already separates issues; what a key must not contain is the run id,
/// which the test above pins.
#[tokio::test(flavor = "multi_thread")]
#[allow(non_snake_case)] // reason: the name is the spec's contract (a CONTROL test)
async fn CONTROL_a_different_issue_is_not_suppressed_by_the_first_ones_keys() {
    let dir = scratch("github-issue-other-");
    let planner = planner_fixture(dir.path(), &[HOUSE_SPEC, HOUSE_SPEC]);
    let (d, github) = daemon_in(dir.path(), &planner);

    let (first, _) = run_graph(&d.engine, delivery(7, ISSUE_TEXT)).await;
    assert_eq!(first["status"], "succeeded", "{first}");
    let (second, _) = run_graph(&d.engine, delivery(8, ISSUE_TEXT)).await;
    assert_eq!(second["status"], "succeeded", "{second}");
    let sent = github.sent.lock().clone();
    assert_eq!(sent.len(), 4, "two comments for each issue: {sent:?}");
    assert_eq!(
        sent.iter().filter(|(to, _)| to == "8").count(),
        2,
        "{sent:?}"
    );
}

#[test]
fn no_absolute_path_is_written_in_the_graph() {
    let src = graph_src();
    for path in ["/projects", "/root", "/home"] {
        assert!(
            !src.contains(path),
            "the graph may not hard-code {path}: its paths are the workspace's"
        );
    }
}

/// The string entries of a TOML array value, or none when the key is absent.

#[test]
fn the_planner_node_holds_no_spawn_or_proc_and_only_reads() {
    let src = graph_src();
    let doc: toml::Table = toml::from_str(&src).expect("valid toml");
    let graph_caps = string_list(doc.get("caps"));
    assert!(
        !graph_caps.iter().any(|c| c == "spawn"),
        "the graph does not grant spawn: {graph_caps:?}"
    );

    let all = nodes(&src);
    let plan = all
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("plan"))
        .expect("a plan node");
    let plan_caps = string_list(plan.get("caps"));
    assert!(
        !plan_caps.iter().any(|c| c == "spawn" || c == "proc"),
        "the plan node holds neither spawn nor proc: {plan_caps:?}"
    );
    assert_eq!(
        string_list(plan.get("tools")),
        vec!["fs_read".to_owned()],
        "the plan node only reads"
    );

    for n in &all {
        let id = n.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let caps = string_list(n.get("caps"));
        assert!(
            !caps.iter().any(|c| c == "spawn" || c == "proc"),
            "node {id} holds spawn or proc in its own caps: {caps:?}"
        );
    }
}
