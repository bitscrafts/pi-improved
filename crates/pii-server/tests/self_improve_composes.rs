//! Spec 363 R4: `graphs/self-improve.toml` composes `plan` and `enqueue` with the queue graph it calls by pin, and invents
//! no node kind. The graph is loaded the way `pii graph lint` loads it, with the lint registry.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use pii_graph::fragment;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph(name: &str) -> String {
    std::fs::read_to_string(repo().join(format!("graphs/{name}.toml")))
        .unwrap_or_else(|e| panic!("graphs/{name}.toml: {e}"))
}

fn kinds(src: &str) -> BTreeSet<String> {
    let doc: toml::Table = toml::from_str(src).expect("valid toml");
    doc.get("node")
        .and_then(|n| n.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|n| n.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_flow_is_a_composition() {
    let src = graph("self-improve");
    let reg = pii_server::lint_registry(Path::new("."), Path::new("."));
    let woven = fragment::load_host_with_graphs(
        &src,
        &repo().join("graphs/fragments"),
        std::slice::from_ref(&repo().join("graphs")),
        &reg,
    )
    .expect("the self-improve flow lints");

    let queue = graph("queue");
    let pin = fragment::sha256_hex(queue.as_bytes());
    let call = woven
        .calls
        .iter()
        .find(|c| c.call == "queue")
        .unwrap_or_else(|| panic!("no call record for `queue`: {:?}", woven.calls));
    assert_eq!(
        call.sha256, pin,
        "the pin is the sha256 of graphs/queue.toml on disk"
    );

    let mut allowed: BTreeSet<String> = ["transform", "call", "agent", "ledger"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    allowed.extend(kinds(&queue));
    let invented: Vec<String> = kinds(&src).difference(&allowed).cloned().collect();
    assert!(
        invented.is_empty(),
        "the flow uses kinds nothing offers: {invented:?}"
    );
}
