//! Spec 215: a grant ceiling is the most capability any node may request. A graph asking beyond it is refused at
//! load with `graph.grant.ceiling`, naming the node and the ceiling. The ceiling is never widened to admit a graph.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_core::CapKind;
use pii_graph::NodeRegistry;

const SPAWNING_GRAPH: &str = "name = \"spawner\"\nentry = \"code\"\ncaps = [\"spawn\"]\n\n[[node]]\nid = \"code\"\nkind = \"transform\"\ncaps = [\"spawn\"]\n";

/// With no ceiling set, the graph loads as before.
#[test]
fn no_ceiling_admits_the_graph() {
    let reg = NodeRegistry::builtin();
    assert!(pii_graph::load_toml_with_registry(SPAWNING_GRAPH, &reg).is_ok());
}

/// A node asking for `spawn` under a ceiling without `spawn` is refused, and the message names the node, the
/// capability and the ceiling.
#[test]
fn node_beyond_the_ceiling_is_refused_and_named() {
    let src = "name = \"spawner\"\nentry = \"code\"\n\n[[node]]\nid = \"code\"\nkind = \"transform\"\ncaps = [\"spawn\"]\n";
    let reg = NodeRegistry::builtin().with_grant_ceiling(CapKind::FS_READ | CapKind::PROC);
    let err = pii_graph::load_toml_with_registry(src, &reg)
        .expect_err("a node beyond the ceiling must not load");
    let msg = err.to_string();
    assert!(msg.contains("graph.grant.ceiling"), "{msg}");
    assert!(msg.contains("'code'"), "names the node: {msg}");
    assert!(msg.contains("spawn"), "names the capability: {msg}");
    assert!(msg.contains("fs_read"), "names the ceiling: {msg}");
}

/// A graph-level `caps` entry beyond the ceiling is refused the same way.
#[test]
fn graph_caps_beyond_the_ceiling_are_refused() {
    let src = "name = \"net-graph\"\nentry = \"a\"\ncaps = [\"net\"]\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\n";
    let reg = NodeRegistry::builtin().with_grant_ceiling(CapKind::FS_READ);
    let err =
        pii_graph::load_toml_with_registry(src, &reg).expect_err("graph caps beyond the ceiling");
    assert!(err.to_string().contains("graph.grant.ceiling"), "{err}");
}

/// A graph whose every request sits within the ceiling loads.
#[test]
fn graph_within_the_ceiling_loads() {
    let reg = NodeRegistry::builtin().with_grant_ceiling(CapKind::SPAWN);
    assert!(pii_graph::load_toml_with_registry(SPAWNING_GRAPH, &reg).is_ok());
}
