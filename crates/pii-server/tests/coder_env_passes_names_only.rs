//! Spec 207: a `coder` node's child gets exactly the `env_pass` names from the daemon's environment. Every other
//! variable, secrets included, is absent; values are never written to the graph.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome};
use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_observe::NullObserver;
use pii_server::node_kinds::nodes::coder::CoderFactory;

/// The fake `pi` writes its exported environment (a shell builtin, so no PATH is needed) into its workdir.
fn fake_pi_dumping_env(bin: &std::path::Path) {
    let pi = bin.join("pi");
    std::fs::write(&pi, "#!/bin/sh\nexport -p > \"$PWD/env.out\"\n").unwrap();
    std::fs::set_permissions(&pi, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

#[test]
fn child_sees_only_the_named_env_pass_variables() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    fake_pi_dumping_env(bin.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();

    // The daemon's environment, as the factory snapshots it: one passed name, one secret not passed.
    let daemon_env = BTreeMap::from([
        (String::from("PII_TEST_KEY"), String::from("passed-value")),
        (String::from("OTHER_API_KEY"), String::from("secret-value")),
    ]);
    let factory =
        CoderFactory::with_env(bin.path().to_path_buf(), daemon_env, Arc::new(NullObserver));
    let cfg = serde_json::json!({
        "agent": "pi",
        "model": "m",
        "brief": "b",
        "spec": "specs/207-x.md",
        "workdir": work.path().display().to_string(),
        "criteria": ["test -f env.out"],
        "env_pass": ["PII_TEST_KEY"],
    });
    let node = pii_graph::node::NodeFactory::build(&factory, &cfg).unwrap();

    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![
        String::from("/bin/sh"),
        bin.path().join("pi").display().to_string(),
    ]);
    let ctx = Ctx::root(caps, Budget::unlimited());
    let input = NodeInput {
        input: serde_json::json!({}),
        payload: serde_json::json!({}),
        nodes: serde_json::json!({}),
    };
    let outcome = node.run(&ctx, &input).unwrap();
    assert!(matches!(outcome, Outcome::Next { .. }), "got {outcome:?}");

    let dump = std::fs::read_to_string(work.path().join("env.out")).unwrap();
    assert!(
        // `export -p` quotes values (`export NAME='value'`): match the name and the value on one line.
        dump.lines()
            .any(|l| l.contains("PII_TEST_KEY") && l.contains("passed-value")),
        "named var passed: {dump}"
    );
    assert!(
        !dump.contains("OTHER_API_KEY"),
        "unnamed secret absent: {dump}"
    );
    assert!(
        !dump.contains("secret-value"),
        "secret value absent: {dump}"
    );
}
