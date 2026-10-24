//! Spec 256: a `call` node's `caps` may not grant more than the graph it sits in holds. The per-node rule (a
//! sub-graph's node needs no more than its call grants) is tested in `call_inlines_a_graph.rs`; these tests pin the
//! call-versus-host rule on its own, so the sub-graph needs only what the call grants and the refusal comes from the call.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_graph::fragment::{self, Woven};
use pii_graph::node::NodeRegistry;

/// A sub-graph whose one node needs only `proc`.
const SUB: &str = r#"
name = "docs"
exit = "a"

[[node]]
id = "a"
kind = "transform"
expr = "$.input.title"
caps = ["proc"]
"#;

/// The sub-graph with no caps declared anywhere.
const SUB_NO_CAPS: &str = r#"
name = "docs"
exit = "a"

[[node]]
id = "a"
kind = "transform"
expr = "$.input.title"
"#;

/// The host: `start -> docs(call) -> done`. `PIN` is the sub-graph's real digest; `HOST_CAPS` and `CALL_CAPS` are
/// filled per test.
const HOST: &str = r#"
name = "host"
entry = "start"
caps = HOST_CAPS

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "docs"
sha256 = "PIN"
caps = CALL_CAPS

[node.in]
title = "$.input.x"

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "start"
port = "ok"
to = "docs"
kind = "data"

[[edge]]
from = "docs"
port = "ok"
to = "done"
kind = "data"
"#;

/// Writes the named graph files under a fresh temp dir and returns it (kept alive by the caller) with the paths.
struct Dirs {
    _root: tempfile::TempDir,
    graphs: PathBuf,
    fragments: PathBuf,
}

fn dirs(graphs: &[(&str, &str)]) -> Dirs {
    let root = tempfile::tempdir().expect("tempdir");
    let graph_dir = root.path().join("graphs");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    for (name, src) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), src).expect("write graph");
    }
    let fragments = root.path().join("fragments");
    Dirs {
        _root: root,
        graphs: graph_dir,
        fragments,
    }
}

fn pin(src: &str) -> String {
    fragment::sha256_hex(src.as_bytes())
}

/// The host with its caps and call caps set, and its `PIN` replaced by the real digest of `sub`.
fn host_with(sub: &str, host_caps: &str, call_caps: &str) -> String {
    HOST.replace("PIN", &pin(sub))
        .replace("HOST_CAPS", host_caps)
        .replace("CALL_CAPS", call_caps)
}

fn load(host: &str, graphs: &[(&str, &str)]) -> Result<Woven, PiiError> {
    let d = dirs(graphs);
    fragment::load_host_with_graphs(
        host,
        &d.fragments,
        std::slice::from_ref(&d.graphs),
        &NodeRegistry::builtin(),
    )
}

fn load_err(host: &str, graphs: &[(&str, &str)]) -> String {
    load(host, graphs)
        .expect_err("load must refuse")
        .to_string()
}

#[test]
fn a_call_cannot_grant_more_than_its_host_holds() {
    let host = host_with(SUB, "[\"proc\"]", "[\"proc\", \"spawn\"]");
    let msg = load_err(&host, &[("docs", SUB)]);
    assert!(msg.contains("spawn"), "names the capability: {msg}");
    assert!(msg.contains("'docs'"), "names the call node: {msg}");
}

#[test]
fn a_call_within_its_host_grant_loads() {
    let host = host_with(SUB, "[\"proc\", \"spawn\"]", "[\"proc\", \"spawn\"]");
    assert!(load(&host, &[("docs", SUB)]).is_ok());
}

#[test]
fn the_call_ceiling_does_not_depend_on_the_subgraph() {
    let host = host_with(SUB_NO_CAPS, "[\"proc\"]", "[\"proc\", \"spawn\"]");
    let msg = load_err(&host, &[("docs", SUB_NO_CAPS)]);
    assert!(msg.contains("spawn"), "names the capability: {msg}");
}
