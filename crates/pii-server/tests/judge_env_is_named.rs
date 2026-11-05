//! A `judge` criterion runs in a scrubbed environment: the daemon's own variables never reach it. A cargo criterion
//! needs a target dir the sandbox lets it write, so the judge's config may NAME the few variables it forwards:
//! `env = ["CARGO_TARGET_DIR"]` copies that one variable from the daemon. Measured 2026-11-05: without it every
//! cargo criterion ran with no target dir, could not finish inside the judge, and read as red, which is exactly
//! what the RED step wants, so nothing noticed for a night.
//!
//! The list is closed: a name that is not on it, and every secret-named variable, is refused at LOAD naming it.

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

const SPEC: &str = "specs/900-env.md";

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

fn repo_with(criterion: &str) -> (tempfile::TempDir, String) {
    let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "judge-env-").unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::create_dir_all(root.join("specs")).unwrap();
    std::fs::write(
        root.join(SPEC),
        format!("## Exit Criteria\n\n- [ ] `{criterion}`\n"),
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "spec"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    (dir, base)
}

fn cfg(workdir: &Path, base: &str, env: Option<serde_json::Value>) -> serde_json::Value {
    let mut c = serde_json::json!({
        "spec": SPEC, "base": base, "workdir": workdir.display().to_string(),
    });
    if let (Some(env), Some(map)) = (env, c.as_object_mut()) {
        map.insert("env".to_owned(), env);
    }
    c
}

fn verdict(cfg: &serde_json::Value) -> serde_json::Value {
    let node = JudgeFactory.build(cfg).unwrap();
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    match node.run(&ctx(), &input).unwrap() {
        Outcome::Next { value, .. } => value,
        other => panic!("judge yields a port: {other:?}"),
    }
}

/// `PATH` is on the scrub's own allowlist and on the judge's, so it is always set: a criterion that needs a
/// variable the daemon has but the scrub drops can only pass if the judge forwarded it. `CARGO_HOME` is set in
/// the test runner's environment by cargo itself, so it is present to forward.
const FORWARDED: &str = "CARGO_HOME";

fn have_forwardable() -> bool {
    pii_core::env_var(FORWARDED).is_some_and(|v| !v.is_empty())
}

#[test]
fn a_named_variable_reaches_the_criterion() {
    assert!(have_forwardable(), "cargo sets {FORWARDED} for a test run");
    let (repo, base) = repo_with(&format!("test -n \"${FORWARDED}\""));
    let v = verdict(&cfg(
        repo.path(),
        &base,
        Some(serde_json::json!([FORWARDED])),
    ));
    assert_eq!(v["verdict"], "passed", "{v}");
}

#[test]
fn control_an_unnamed_variable_stays_scrubbed() {
    // The near miss: the same criterion, the same variable present in the daemon, NOT named in `env`.
    assert!(have_forwardable(), "cargo sets {FORWARDED} for a test run");
    let (repo, base) = repo_with(&format!("test -n \"${FORWARDED}\""));
    let v = verdict(&cfg(repo.path(), &base, None));
    assert_ne!(v["verdict"], "passed", "the scrub must still drop it: {v}");
}

#[test]
fn control_naming_a_variable_that_is_not_set_does_not_invent_one() {
    let (repo, base) = repo_with("test -z \"$CARGO_BUILD_JOBS\"");
    // CARGO_BUILD_JOBS is on the closed list but unset in this process: nothing is forwarded, nothing invented.
    if pii_core::env_var("CARGO_BUILD_JOBS").is_some() {
        return;
    }
    let v = verdict(&cfg(
        repo.path(),
        &base,
        Some(serde_json::json!(["CARGO_BUILD_JOBS"])),
    ));
    assert_eq!(v["verdict"], "passed", "{v}");
}

#[test]
fn a_name_off_the_list_is_refused_at_load_naming_it() {
    let err = JudgeFactory
        .build(&cfg(
            Path::new("/tmp"),
            "x",
            Some(serde_json::json!(["HOME_SECRET_THING"])),
        ))
        .err()
        .expect("refused")
        .to_string();
    assert!(err.contains("HOME_SECRET_THING"), "{err}");
}

#[test]
fn a_secret_named_variable_is_refused_even_if_asked_for() {
    for name in ["OLLAMA_API_KEY", "DISCORD_BOT_TOKEN", "PII_PASSWORD"] {
        let err = JudgeFactory
            .build(&cfg(
                Path::new("/tmp"),
                "x",
                Some(serde_json::json!([name])),
            ))
            .err()
            .expect("refused")
            .to_string();
        assert!(err.contains(name), "{err}");
    }
}
