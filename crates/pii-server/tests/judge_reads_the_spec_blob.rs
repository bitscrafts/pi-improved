//! Spec 259: `judge` in spec mode reads a spec's Exit Criteria from its committed blob, runs every one, and reports each
//! result. Each test builds a throwaway git repo, then RUNS the node; the working tree may differ from the blob.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::process::Command;

use pii_abi::{NodeInput, Outcome};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_graph::node::NodeFactory;
use pii_server::node_kinds::nodes::queue::JudgeFactory;

const SPEC: &str = "specs/900-judged.md";

/// A root context whose only argv0 is `/bin/sh`, with the proc capability the judge needs.
fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
    Ctx::root(caps, Budget::unlimited())
}

// reason: test fixture only; it builds the repo the node reads, and no production path runs through it
#[allow(clippy::disallowed_methods)]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A repo whose one commit holds `SPEC` with `blob` as its text. Returns the repo dir and the commit id.
fn repo_with_spec(blob: &str) -> (tempfile::TempDir, String) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "judge-spec-").unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::create_dir_all(root.join("specs")).unwrap();
    std::fs::write(root.join(SPEC), blob).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "spec"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    (dir, base)
}

/// The judge config in spec mode: `spec`, `base` and `workdir` are literals here.
fn spec_cfg(workdir: &Path, spec: &str, base: &str) -> serde_json::Value {
    serde_json::json!({
        "spec": spec,
        "base": base,
        "workdir": workdir.display().to_string(),
    })
}

/// Runs the judge built from `cfg` over `input`, returning the outcome's port name and payload.
fn judge(cfg: &serde_json::Value, input: serde_json::Value) -> (String, serde_json::Value) {
    let node = JudgeFactory.build(cfg).unwrap();
    let input = NodeInput {
        input: input.clone(),
        payload: input,
        nodes: serde_json::json!({}),
    };
    match node.run(&ctx(), &input).unwrap() {
        Outcome::Next { port, value } => (port.to_string(), value),
        other => panic!("judge yields a port: {other:?}"),
    }
}

/// The payload of a judge run in spec mode whose config is literal.
fn judge_literal(workdir: &Path, spec: &str, base: &str) -> (String, serde_json::Value) {
    judge(&spec_cfg(workdir, spec, base), serde_json::json!({}))
}

/// The commands of a `results` array, in order.
fn commands(value: &serde_json::Value) -> Vec<String> {
    value["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["criterion"].as_str().unwrap().to_owned())
        .collect()
}

/// A criterion that writes `marker` into the working directory when it runs.
fn marker_cmd(marker: &str) -> String {
    format!("touch {marker}")
}

#[test]
fn every_criterion_of_the_blob_runs_and_all_pass() {
    let blob = "# Spec\n\n## Exit Criteria\n\n- [ ] `true`\n- [ ] `test 1 -eq 1`\n- [ ] `exit 0`\n";
    let (repo, base) = repo_with_spec(blob);
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "ok", "{value}");
    assert_eq!(value["verdict"], "passed");
    assert_eq!(value["criteria"], 3);
    assert_eq!(value["passed"], 3);
    assert_eq!(value["results"].as_array().unwrap().len(), 3);
}

#[test]
fn a_failure_does_not_stop_the_rest() {
    let marker = "after-the-failure.txt";
    let blob = format!(
        "## Exit Criteria\n\n- [ ] `true`\n- [ ] `false`\n- [ ] `{}`\n",
        marker_cmd(marker)
    );
    let (repo, base) = repo_with_spec(&blob);
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "fail", "{value}");
    assert_eq!(value["results"].as_array().unwrap().len(), 3, "{value}");
    assert_eq!(value["failing"], serde_json::json!(["false"]), "{value}");
    assert_eq!(value["passed"], 2, "{value}");
    assert!(repo.path().join(marker).exists(), "the third criterion ran");
}

#[test]
fn the_working_tree_spec_is_never_read() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `false`\n");
    // The working tree now says the criterion passes; the blob must still win.
    std::fs::write(repo.path().join(SPEC), "## Exit Criteria\n\n- [ ] `true`\n").unwrap();
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "fail", "{value}");
    assert_eq!(value["failing"], serde_json::json!(["false"]), "{value}");
}

#[test]
fn an_edit_that_adds_a_criterion_is_ignored() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `true`\n");
    // A worker adds a failing criterion to its own spec: the blob has no such line, so it is not run.
    std::fs::write(
        repo.path().join(SPEC),
        "## Exit Criteria\n\n- [ ] `true`\n- [ ] `false`\n",
    )
    .unwrap();
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "ok", "{value}");
    assert_eq!(value["criteria"], 1, "{value}");
}

#[test]
fn a_non_commit_base_is_refused_and_nothing_runs() {
    let marker = "must-not-exist.txt";
    let blob = format!("## Exit Criteria\n\n- [ ] `{}`\n", marker_cmd(marker));
    let (repo, _base) = repo_with_spec(&blob);
    for base in ["HEAD", "a1b2c3d", ""] {
        let (port, value) = judge_literal(repo.path(), SPEC, base);
        assert_eq!(port, "fail", "base {base:?}: {value}");
        assert!(
            value["reason"].as_str().unwrap().contains("base"),
            "base {base:?}: {value}"
        );
        assert!(
            !repo.path().join(marker).exists(),
            "base {base:?}: nothing runs"
        );
    }
}

#[test]
fn a_real_40_hex_base_is_accepted() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `true`\n");
    assert_eq!(base.len(), 40, "a full commit id: {base}");
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "ok", "{value}");
}

#[test]
fn an_unknown_spec_path_fails_naming_it() {
    let marker = "unknown-spec.txt";
    let (repo, base) = repo_with_spec(&format!(
        "## Exit Criteria\n\n- [ ] `{}`\n",
        marker_cmd(marker)
    ));
    let (port, value) = judge_literal(repo.path(), "specs/nope.md", &base);
    assert_eq!(port, "fail", "{value}");
    let reason = value["reason"].as_str().unwrap();
    assert!(reason.contains("specs/nope.md"), "{reason}");
    assert!(reason.contains(&base), "{reason}");
    assert!(!repo.path().join(marker).exists(), "nothing runs");
}

#[test]
fn a_spec_without_exit_criteria_fails() {
    let (repo, base) = repo_with_spec("# Spec\n\n## Goals\n\n- [ ] `true`\n");
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "fail", "{value}");
    assert!(
        value["reason"]
            .as_str()
            .unwrap()
            .contains("has no Exit Criteria"),
        "{value}"
    );
}

#[test]
fn the_section_ends_at_the_next_heading() {
    let marker = "guardrail-line.txt";
    let blob = format!(
        "## Exit Criteria\n\n- [ ] `true`\n\n## Guardrails\n\n- [ ] `{}`\n",
        marker_cmd(marker)
    );
    let (repo, base) = repo_with_spec(&blob);
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "ok", "{value}");
    assert_eq!(value["criteria"], 1, "{value}");
    assert!(
        !repo.path().join(marker).exists(),
        "the guardrail line is not run"
    );
}

#[test]
fn a_checked_box_is_a_criterion_too() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [x] `false`\n");
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "fail", "{value}");
    assert_eq!(commands(&value), ["false"], "{value}");
}

#[test]
fn only_the_first_backticked_span_is_the_command() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `true` and then `false`\n");
    let (port, value) = judge_literal(repo.path(), SPEC, &base);
    assert_eq!(port, "ok", "{value}");
    assert_eq!(commands(&value), ["true"], "{value}");
}

#[test]
fn input_keys_resolve_from_the_run_input() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `true`\n");
    let cfg = serde_json::json!({
        "spec": "$.input.spec",
        "base": "$.input.base",
        "workdir": "$.input.workdir",
    });
    let input = serde_json::json!({
        "spec": SPEC,
        "base": base,
        "workdir": repo.path().display().to_string(),
    });
    let (port, value) = judge(&cfg, input);
    assert_eq!(port, "ok", "{value}");
    assert_eq!(value["spec"], SPEC, "{value}");
    assert_eq!(value["base"], base.as_str(), "{value}");
}

#[test]
fn item_mode_is_untouched() {
    let repo = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "judge-item-").unwrap();
    let cfg = serde_json::json!({ "workdir": repo.path().display().to_string() });
    let input = serde_json::json!({
        "nodes": { "select": { "item": { "id": "A1", "criteria": ["true", "false"] } } }
    });
    let node_input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: input["nodes"].clone(),
    };
    let node = JudgeFactory.build(&cfg).unwrap();
    let Outcome::Next { port, value } = node.run(&ctx(), &node_input).unwrap() else {
        panic!("item mode yields a port");
    };
    assert_eq!(port.to_string(), "fail");
    assert_eq!(value["criterion"], "false", "{value}");
    assert_eq!(value["rc"], 1, "{value}");
}

#[test]
fn a_criterion_text_cannot_inject_through_the_arguments() {
    let marker = "injected.txt";
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `true`\n");
    let hostile = format!("; touch {marker}; ");
    let (port, value) = judge_literal(repo.path(), &hostile, &base);
    assert_eq!(port, "fail", "{value}");
    assert!(
        !repo.path().join(marker).exists(),
        "the argument is not a command"
    );
}
