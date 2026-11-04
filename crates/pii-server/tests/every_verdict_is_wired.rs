//! Spec 381: every verdict a node can produce is wired to a port the graph routes on.
//!
//! R1 lives here: `graphs/tdd.toml` lints with no V9 (a multi-arm join with too few inbound data edges). R2 and R3 have
//! their own tests in `queue_calls_spec.rs` and `night_calls_sdd.rs`, next to the graphs they check.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::PathBuf;

use pii_graph::fragment;

fn graphs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

#[test]
fn tdd_graph_lints_with_no_v9() {
    // The registry `pii graph lint` uses, so `coder` and `judge` are known kinds.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reg = pii_server::lint_registry(&root, &root);
    let src = std::fs::read_to_string(graphs_dir().join("tdd.toml")).unwrap();
    let frags = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "evw-frags-").unwrap();
    let woven = fragment::load_host_with_graphs(
        &src,
        frags.path(),
        std::slice::from_ref(&graphs_dir()),
        &reg,
    )
    .expect("graphs/tdd.toml loads");
    let warnings = pii_graph::validate(&woven.spec, &reg).expect("graphs/tdd.toml validates");
    let v9: Vec<&String> = warnings.iter().filter(|w| w.starts_with("V9")).collect();
    assert!(v9.is_empty(), "a one-armed join is still V9: {v9:?}");
}
