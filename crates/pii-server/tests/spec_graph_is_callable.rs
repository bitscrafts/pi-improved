//! Spec 364 R1: `graphs/spec.toml` can be called. It names its exit, and the capability ladder covers its coder.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use pii_graph::fragment;
use pii_graph::node::NodeRegistry;

fn graphs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

/// Loads a host holding `caps` that calls the tracked spec graph with that same grant.
fn host_with(caps: &str) -> Result<fragment::Woven, pii_abi::PiiError> {
    let src = std::fs::read(graphs_dir().join("spec.toml")).unwrap();
    let pin = fragment::sha256_hex(&src);
    let host = format!(
        "name = \"host\"\nentry = \"implement\"\nexit = \"implement\"\ncaps = [{caps}]\n\n\
         [[node]]\nid = \"implement\"\nkind = \"call\"\ngraph = \"spec\"\nsha256 = \"{pin}\"\ncaps = [{caps}]\n"
    );
    fragment::load_host_with_graphs(
        &host,
        &graphs_dir().join("fragments"),
        &[graphs_dir()],
        &NodeRegistry::builtin(),
    )
}

#[test]
fn spec_declares_an_exit_and_its_caps_cover_its_coder() {
    let src = std::fs::read_to_string(graphs_dir().join("spec.toml")).unwrap();
    let doc: toml::Table = toml::from_str(&src).unwrap();
    assert_eq!(doc["exit"].as_str(), Some("done"));

    let woven = host_with(r#""spawn", "proc", "fs_read", "fs_write""#)
        .expect("a host holding spawn,proc,fs_read,fs_write may call spec");
    let code = woven
        .spec
        .nodes
        .iter()
        // spec 385 put the implementer behind the methodology call, so the coder is namespaced one level
        // deeper: implement → method-judge → code.
        .find(|n| n.id.as_str() == "implement__method-judge__code")
        .expect("the callee's coder is inlined");
    assert_eq!(code.kind, "coder");

    let Err(refused) = host_with(r#""spawn", "proc", "fs_read""#) else {
        panic!("a host without fs_write must not be able to call spec");
    };
    let refused = refused.to_string();
    assert!(refused.contains("graph.call.widen"), "{refused}");
    assert!(refused.contains("fs_write"), "names the cap: {refused}");
}
