//! Spec 372: `judge` with `expect = "fail"` is the RED step of a TDD loop. Its `ok` port means every criterion failed,
//! its `fail` port means some criterion passed before the code existed (`not-red`), and a criterion that could not run
//! at all (exit 127) gets its own `unrunnable` port. Without `expect`, the judge keeps its pass meaning.

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

const SPEC: &str = "specs/900-expected.md";

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
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "judge-expect-").unwrap();
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

/// The judge config in spec mode, with `expect` only when the test sets one.
fn spec_cfg(workdir: &Path, base: &str, expect: Option<&str>) -> serde_json::Value {
    let mut cfg = serde_json::json!({
        "spec": SPEC,
        "base": base,
        "workdir": workdir.display().to_string(),
    });
    if let (Some(expect), Some(map)) = (expect, cfg.as_object_mut()) {
        map.insert("expect".to_owned(), expect.into());
    }
    cfg
}

/// Runs the judge built from `cfg`, returning the outcome's port name and payload.
fn judge(cfg: &serde_json::Value) -> (String, serde_json::Value) {
    let node = JudgeFactory.build(cfg).unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    match node.run(&ctx(), &input).unwrap() {
        Outcome::Next { port, value } => (port.to_string(), value),
        other => panic!("judge yields a port: {other:?}"),
    }
}

#[test]
fn every_criterion_failing_is_red_and_ok() {
    let blob = "## Exit Criteria\n\n- [ ] `false`\n- [ ] `test 1 -eq 2`\n";
    let (repo, base) = repo_with_spec(blob);
    let (port, value) = judge(&spec_cfg(repo.path(), &base, Some("fail")));
    assert_eq!(port, "ok", "{value}");
    assert_eq!(value["verdict"], "red", "{value}");
    assert_eq!(value["criteria"], 2, "{value}");
}

#[test]
fn a_criterion_that_passes_before_the_code_is_not_red() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `false`\n- [ ] `true`\n");
    let (port, value) = judge(&spec_cfg(repo.path(), &base, Some("fail")));
    assert_eq!(port, "fail", "{value}");
    assert_eq!(value["verdict"], "not-red", "{value}");
    assert_eq!(value["passing"], serde_json::json!(["true"]), "{value}");
    assert!(
        value["reason"].as_str().unwrap().contains("true"),
        "{value}"
    );
}

#[test]
fn a_criterion_that_cannot_run_is_its_own_port_not_red() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `no-such-binary-372 --flag`\n");
    let (port, value) = judge(&spec_cfg(repo.path(), &base, Some("fail")));
    assert_eq!(port, "unrunnable", "{value}");
    assert_eq!(value["verdict"], "unrunnable", "{value}");
}

#[test]
fn unrunnable_wins_over_a_passing_criterion() {
    let (repo, base) =
        repo_with_spec("## Exit Criteria\n\n- [ ] `true`\n- [ ] `no-such-binary-372`\n");
    let (port, value) = judge(&spec_cfg(repo.path(), &base, Some("fail")));
    assert_eq!(port, "unrunnable", "{value}");
}

#[test]
fn no_expect_keeps_the_pass_meaning() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `true`\n");
    let (port, value) = judge(&spec_cfg(repo.path(), &base, None));
    assert_eq!(port, "ok", "{value}");
    assert_eq!(value["verdict"], "passed", "{value}");
}

#[test]
fn expect_pass_missing_binary_is_still_a_plain_fail() {
    let (repo, base) = repo_with_spec("## Exit Criteria\n\n- [ ] `no-such-binary-372`\n");
    let (port, value) = judge(&spec_cfg(repo.path(), &base, None));
    assert_eq!(port, "fail", "{value}");
    assert_eq!(value["verdict"], "failed", "{value}");
}

#[test]
fn expect_fail_declares_the_unrunnable_port() {
    let cfg = spec_cfg(Path::new("/unused"), &"0".repeat(40), Some("fail"));
    let ports: Vec<String> = JudgeFactory
        .ports(&cfg)
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(ports.contains(&"unrunnable".to_owned()), "{ports:?}");
    let plain = spec_cfg(Path::new("/unused"), &"0".repeat(40), None);
    let plain_ports = JudgeFactory.ports(&plain).unwrap();
    assert!(
        !plain_ports.iter().any(|p| p.to_string() == "unrunnable"),
        "{plain_ports:?}"
    );
}

#[test]
fn an_unknown_expect_value_is_refused_at_build() {
    let cfg = spec_cfg(Path::new("/unused"), &"0".repeat(40), Some("maybe"));
    assert!(JudgeFactory.build(&cfg).is_err());
}
