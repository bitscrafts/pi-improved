//! Spec 403 (the fan-out over rubric slots): split out of `review_graph.rs`, which had grown past the file-size limit.

#![allow(clippy::wildcard_imports)] // reason: the shared fakes and helpers live in the parent module

use super::*;

// ---- R5: the fan-out ----

#[tokio::test]
async fn review_with_all_slots_passing_ends_ok() {
    let (_dir, wd) = workdir(&[
        ("alpha", RUBRIC_BODY),
        ("beta", RUBRIC_BODY),
        ("gamma", RUBRIC_BODY),
    ]);
    let mut input = base(&wd);
    input["rubric_1"] = serde_json::json!("alpha");
    input["rubric_2"] = serde_json::json!("beta");
    input["rubric_3"] = serde_json::json!("gamma");
    let settled = run("review", &passes(), &input).await;
    assert!(settled.passed(), "{:?}", settled.text());
}

/// Slot 2's reviewer is refused by its rubric being absent, not by a FAIL answer: the scripted answers are consumed in
/// call order and the slots run concurrently, so a FAIL answer cannot be pinned to slot 2.
#[tokio::test]
async fn review_with_one_failing_slot_names_that_rubric_only() {
    let (_dir, wd) = workdir(&[("alpha", RUBRIC_BODY), ("gamma", RUBRIC_BODY)]);
    let mut input = base(&wd);
    input["rubric_1"] = serde_json::json!("alpha");
    input["rubric_2"] = serde_json::json!("omega-missing");
    input["rubric_3"] = serde_json::json!("gamma");
    let settled = run("review", &passes(), &input).await;
    let text = settled.failures();
    assert!(!settled.passed(), "{}", settled.text());
    assert!(text.contains("omega-missing"), "names rubric 2: {text}");
    assert!(!text.contains("alpha"), "does not name rubric 1: {text}");
    assert!(!text.contains("gamma"), "does not name rubric 3: {text}");
}

#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names its controls with the CONTROL_ prefix
async fn CONTROL_a_skipped_slot_is_not_a_failure() {
    let (_dir, wd) = workdir(&[("alpha", RUBRIC_BODY)]);
    let mut input = base(&wd);
    input["rubric_1"] = serde_json::json!("alpha");
    let settled = run("review", &passes(), &input).await;
    assert!(settled.passed(), "{:?}", settled.text());
}

#[tokio::test]
async fn review_without_rubric_1_is_refused() {
    let (_dir, wd) = workdir(&[("alpha", RUBRIC_BODY)]);
    let settled = run("review", &passes(), &base(&wd)).await;
    assert!(!settled.passed(), "{:?}", settled.text());
}

// ---- The vocabulary fence ----

#[test]
#[allow(clippy::disallowed_methods)] // reason: the test drives the same `cargo run` the gate uses, against this tree
fn the_graphs_load_under_the_vocabulary_fence() {
    let out = std::process::Command::new("cargo")
        .args(["run", "-q", "-p", "xtask", "--", "lint-graph-kinds"])
        .current_dir(repo())
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    assert!(text.contains("0 specialised"), "{text}");
}
