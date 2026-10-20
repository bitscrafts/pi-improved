//! Spec 232 R1: the workspace criterion `cargo:test:workspace/all` is scope-checked - the node's
//! `workdir` must be the run's scope root (carried in the run input's `scope_root`). A graph naming
//! it with any other workdir refuses; the checked name resolves to its fixed argv in THAT workdir.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome};
use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_observe::NullObserver;
use pii_server::node_kinds::nodes::coder::CoderFactory;

/// The fake `pi` exits 0 immediately.
fn fake_pi(bin: &std::path::Path) {
    let pi = bin.join("pi");
    std::fs::write(&pi, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

/// A criterion that touches a file in its workdir: the control for "the argv ran in the node's workdir".
fn ran_file_criteria() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(
        String::from("cargo:test:workspace/all"),
        vec![
            String::from("/usr/bin/touch"),
            String::from("criterion-ran.txt"),
        ],
    )])
}

fn input_with_scope(scope: Option<&str>) -> NodeInput {
    NodeInput {
        input: scope.map_or_else(
            || serde_json::json!({}),
            |s| serde_json::json!({ "scope_root": s }),
        ),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    }
}

fn node(bin: &std::path::Path, workdir: &str) -> Box<dyn pii_graph::GraphNode> {
    let factory =
        CoderFactory::with_env(bin.to_path_buf(), BTreeMap::new(), Arc::new(NullObserver))
            .with_criteria(ran_file_criteria());
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "m",
        "brief": "b",
        "spec": "specs/232-x.md",
        "workdir": workdir,
        "criteria": ["cargo:test:workspace/all"],
    });
    pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap()
}

fn ctx(bin: &std::path::Path) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = pii_core::ArgvScope::new(vec![
        String::from("/bin/sh"),
        bin.join("pi").display().to_string(),
        String::from("/usr/bin/touch"),
    ]);
    Ctx::root(caps, Budget::unlimited())
}

#[test]
fn workspace_criterion_runs_in_the_scoped_workdir() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-bin-").unwrap();
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-work-").unwrap();
    fake_pi(bin.path());
    let n = node(bin.path(), work.path().to_str().unwrap());
    let outcome = n
        .run(
            &ctx(bin.path()),
            &input_with_scope(Some(work.path().to_str().unwrap())),
        )
        .unwrap();
    match outcome {
        Outcome::Next { value, .. } => {
            let verdict = value.to_string();
            assert!(
                verdict.contains("\"rc\":0"),
                "criterion ran and passed: {verdict}"
            );
        }
        other => panic!("workspace criterion on the scope root passes: {other:?}"),
    }
    assert!(
        work.path().join("criterion-ran.txt").exists(),
        "the criterion ran INSIDE the node's workdir"
    );
}

#[test]
fn workspace_criterion_on_a_foreign_workdir_refuses() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-bin2-").unwrap();
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-work2-").unwrap();
    let other =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-other-").unwrap();
    fake_pi(bin.path());
    let n = node(bin.path(), other.path().to_str().unwrap());
    let outcome = n
        .run(
            &ctx(bin.path()),
            &input_with_scope(Some(work.path().to_str().unwrap())),
        )
        .unwrap();
    match outcome {
        Outcome::Fail { error } => {
            assert!(
                error.contains("criteria.unknown") && error.contains("cargo:test:workspace/all"),
                "refusal names the criterion: {error}"
            );
        }
        Outcome::Next { port, .. } => {
            let p = port.as_str().to_owned();
            assert!(
                p == "err",
                "a foreign-workdir workspace criterion routes to err, got port {p}"
            );
        }
        other @ Outcome::Suspend { .. } => {
            panic!("workspace criterion off the scope root refuses: {other:?}")
        }
    }
}

/// Without a `scope_root` in the run input there is nothing to check the workdir against: the
/// absence means "not a heal run", the daemon workspace is the scope, and the criterion resolves
/// the same way. The refusal must never depend on the scope being present.
#[test]
fn workspace_criterion_without_scope_root_resolves() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-bin3-").unwrap();
    let work =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "ws-crit-work3-").unwrap();
    fake_pi(bin.path());
    let n = node(bin.path(), work.path().to_str().unwrap());
    let outcome = n.run(&ctx(bin.path()), &input_with_scope(None)).unwrap();
    assert!(
        matches!(outcome, Outcome::Next { .. } | Outcome::Fail { .. }),
        "resolves either way, never criteria.unknown at this layer: {outcome:?}"
    );
}
