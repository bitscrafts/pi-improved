//! Spec 269: a loaded graph records the calls it expanded. Each `call` node gives one record, in host order, with the
//! graph it inlined, the digest the loader verified, the caps it was granted, its `needs`, its exit and its mapped
//! input keys; a nested call names the call that holds it. A graph with no call has no record, and a pin that does
//! not hold yields no load at all, so no record.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_abi::PiiError;
use pii_graph::fragment::{self, CallRecord, Woven, sha256_hex};
use pii_graph::node::NodeRegistry;

/// The sub-graph `docs`: one transform `a` that reads the mapped `title`; its exit is `a`.
const SUB: &str = r#"name = "docs"
exit = "a"

[[node]]
id = "a"
kind = "transform"
expr = "$.input.title"
"#;

/// A host `start -> docs(call, maps title from input x) -> done`, with `caps` at the top and on the call.
fn host(top_caps: &str, call_caps: &str, graph: &str, sha: &str) -> String {
    format!(
        r#"name = "host"
entry = "start"
caps = [{top_caps}]

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "docs"
kind = "call"
graph = "{graph}"
sha256 = "{sha}"
caps = [{call_caps}]

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
"#
    )
}

/// The host with one `docs` call to `SUB`, pinned to its real digest.
fn host_for(sub: &str) -> String {
    host("", "", "docs", &sha256_hex(sub.as_bytes()))
}

/// Writes the graph files and fragment files under a fresh temp dir (kept alive by the caller) and loads `host`.
fn load_with(
    host: &str,
    graphs: &[(&str, &str)],
    fragments: &[(&str, &str)],
) -> Result<Woven, PiiError> {
    let root = tempfile::tempdir().expect("tempdir");
    let graph_dir = root.path().join("graphs");
    let frag_dir = root.path().join("fragments");
    std::fs::create_dir_all(&graph_dir).expect("mkdir graphs");
    std::fs::create_dir_all(&frag_dir).expect("mkdir fragments");
    for (name, src) in graphs {
        std::fs::write(graph_dir.join(format!("{name}.toml")), src).expect("write graph");
    }
    for (name, src) in fragments {
        std::fs::write(frag_dir.join(format!("{name}.toml")), src).expect("write fragment");
    }
    fragment::load_host_with_graphs(
        host,
        &frag_dir,
        std::slice::from_ref(&graph_dir),
        &NodeRegistry::builtin(),
    )
}

fn load(host: &str, graphs: &[(&str, &str)]) -> Result<Woven, PiiError> {
    load_with(host, graphs, &[])
}

/// A top-level record of the call `call` to `graph`, with the given caps, needs and exit, mapping `input_keys`.
fn record(
    call: &str,
    graph: &str,
    sha: &str,
    caps: &[&str],
    needs: Option<&[&str]>,
    exit: &str,
    input_keys: &[&str],
) -> CallRecord {
    CallRecord {
        call: call.into(),
        graph: graph.into(),
        sha256: sha.into(),
        caps: caps.iter().map(|c| (*c).into()).collect(),
        needs: needs.map(|k| k.iter().map(|c| (*c).into()).collect()),
        exit: exit.into(),
        input_keys: input_keys.iter().map(|c| (*c).into()).collect(),
        parent: None,
    }
}

/// The record `r` with its parent set to the call `parent`, as a nested call is.
fn under(mut r: CallRecord, parent: &str) -> CallRecord {
    r.parent = Some(parent.into());
    r
}

#[test]
fn one_call_one_record() {
    let h = host(
        "\"fs_read\"",
        "\"fs_read\"",
        "docs",
        &sha256_hex(SUB.as_bytes()),
    );
    let w = load(&h, &[("docs", SUB)]).expect("load");
    let sha = sha256_hex(SUB.as_bytes());
    assert_eq!(
        w.calls,
        vec![record(
            "docs",
            "docs",
            &sha,
            &["fs_read"],
            None,
            "a",
            &["title"]
        )]
    );
}

#[test]
fn two_calls_in_host_order() {
    let sha = sha256_hex(SUB.as_bytes());
    let src = format!(
        r#"name = "host"
entry = "start"

[[node]]
id = "start"
kind = "transform"
value = "go"

[[node]]
id = "d1"
kind = "call"
graph = "docs"
sha256 = "{sha}"
caps = []
in = {{ title = "$.input.x" }}

[[node]]
id = "d2"
kind = "call"
graph = "docs"
sha256 = "{sha}"
caps = []
in = {{ title = "$.input.y" }}

[[node]]
id = "done"
kind = "transform"
expr = "$.payload"

[[edge]]
from = "start"
port = "ok"
to = "d1"
kind = "data"

[[edge]]
from = "d1"
port = "ok"
to = "d2"
kind = "data"

[[edge]]
from = "d2"
port = "ok"
to = "done"
kind = "data"
"#
    );
    let w = load(&src, &[("docs", SUB)]).expect("load");
    assert_eq!(
        w.calls,
        vec![
            record("d1", "docs", &sha, &[], None, "a", &["title"]),
            record("d2", "docs", &sha, &[], None, "a", &["title"]),
        ]
    );
}

#[test]
fn a_nested_call_names_its_parent() {
    const INNER: &str = "name = \"inner\"\nexit = \"a\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"deep\"\n";
    const OUTER: &str = "name = \"outer\"\nexit = \"tail\"\n\n[[node]]\nid = \"n\"\nkind = \"call\"\ngraph = \"inner\"\nsha256 = \"INNER\"\ncaps = []\n\n[[node]]\nid = \"tail\"\nkind = \"transform\"\nvalue = \"t\"\n\n[[edge]]\nfrom = \"n\"\nport = \"ok\"\nto = \"tail\"\nkind = \"data\"\n";
    let outer = OUTER.replace("INNER", &sha256_hex(INNER.as_bytes()));
    let h = host("", "", "outer", &sha256_hex(outer.as_bytes()));
    let w = load(&h, &[("outer", &outer), ("inner", INNER)]).expect("load");
    assert_eq!(
        w.calls,
        vec![
            record(
                "docs",
                "outer",
                &sha256_hex(outer.as_bytes()),
                &[],
                None,
                "tail",
                &["title"]
            ),
            under(
                record(
                    "n",
                    "inner",
                    &sha256_hex(INNER.as_bytes()),
                    &[],
                    None,
                    "a",
                    &[]
                ),
                "docs"
            ),
        ]
    );
}

#[test]
fn control_no_call_no_records() {
    let plain = "name = \"plain\"\nentry = \"start\"\n\n[[node]]\nid = \"start\"\nkind = \"transform\"\nvalue = \"go\"\n";
    let w = load(plain, &[]).expect("load");
    assert!(w.calls.is_empty(), "{:?}", w.calls);
}

#[test]
fn control_needs_none_versus_empty() {
    let body = "\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"x\"\n";
    let none = format!("name = \"docs\"\nexit = \"a\"\n{body}");
    let empty = format!("name = \"docs\"\nexit = \"a\"\nneeds = []\n{body}");
    let w_none = load(&host_for(&none), &[("docs", &none)]).expect("load none");
    let w_empty = load(&host_for(&empty), &[("docs", &empty)]).expect("load empty");
    assert_eq!(w_none.calls[0].needs, None);
    assert_eq!(w_empty.calls[0].needs, Some(Vec::new()));
}

#[test]
fn a_call_with_an_attach_keeps_its_record() {
    const GUARD: &str = "name = \"podman-guard\"\n\n[fragment]\nname = \"podman-guard\"\ninputs = [\"$.input.runtime\"]\noutputs = [\"ok\"]\nmay_block = true\n\n[[node]]\nid = \"check\"\nkind = \"transform\"\nvalue = \"ok\"\n";
    let base = host_for(SUB);
    let with_attach = format!(
        "{base}\n[[attach]]\nid = \"guard\"\nfragment = \"podman-guard\"\nat = \"run.start\"\npin = \"{}\"\n",
        sha256_hex(GUARD.as_bytes())
    );
    let w = load_with(&with_attach, &[("docs", SUB)], &[("podman-guard", GUARD)]).expect("load");
    assert_eq!(w.calls.len(), 1, "{:?}", w.calls);
    assert_eq!(w.calls[0].call, "docs");
}

#[test]
fn a_pin_mismatch_yields_no_record() {
    let err = load(&host("", "", "docs", &"0".repeat(64)), &[("docs", SUB)])
        .expect_err("a wrong pin must not load");
    assert!(err.to_string().contains("fragment.pin.mismatch"), "{err}");
}

/// The `call-from` mark on the node `head` of the loaded graph. The mark is what the run journals, so a record that
/// never reaches the head is a call the run does not show.
fn head_mark(w: &Woven, head: &str) -> serde_json::Value {
    let node = w
        .spec
        .nodes
        .iter()
        .find(|n| n.id.as_str() == head)
        .unwrap_or_else(|| panic!("no node {head} in the loaded graph"));
    node.cfg
        .get("call-from")
        .cloned()
        .unwrap_or_else(|| panic!("{head} carries no call-from mark: {:?}", node.cfg))
}

#[test]
fn the_head_carries_the_record_fields() {
    let h = host(
        "\"fs_read\"",
        "\"fs_read\"",
        "docs",
        &sha256_hex(SUB.as_bytes()),
    );
    let w = load(&h, &[("docs", SUB)]).expect("load");
    // No `needs` and no `parent`: a top-level call with no declared needs carries neither, not a null.
    assert_eq!(
        head_mark(&w, "docs__in"),
        serde_json::json!({
            "graph": "docs",
            "sha256": sha256_hex(SUB.as_bytes()),
            "caps": ["fs_read"],
            "exit": "a",
            "input_keys": ["title"],
        })
    );
}

#[test]
fn the_head_carries_needs_when_the_subgraph_declares_them() {
    let sub = SUB.replace("exit = \"a\"\n", "exit = \"a\"\nneeds = [\"title\"]\n");
    let w = load(&host_for(&sub), &[("docs", &sub)]).expect("load");
    assert_eq!(
        head_mark(&w, "docs__in")["needs"],
        serde_json::json!(["title"])
    );
}

#[test]
fn a_nested_head_names_its_parent() {
    const INNER: &str = "name = \"inner\"\nexit = \"a\"\n\n[[node]]\nid = \"a\"\nkind = \"transform\"\nvalue = \"deep\"\n";
    const OUTER: &str = "name = \"outer\"\nexit = \"tail\"\n\n[[node]]\nid = \"n\"\nkind = \"call\"\ngraph = \"inner\"\nsha256 = \"INNER\"\ncaps = []\n\n[[node]]\nid = \"tail\"\nkind = \"transform\"\nvalue = \"t\"\n\n[[edge]]\nfrom = \"n\"\nport = \"ok\"\nto = \"tail\"\nkind = \"data\"\n";
    let outer = OUTER.replace("INNER", &sha256_hex(INNER.as_bytes()));
    let h = host("", "", "outer", &sha256_hex(outer.as_bytes()));
    let w = load(&h, &[("outer", &outer), ("inner", INNER)]).expect("load");
    assert_eq!(
        head_mark(&w, "docs__n__in"),
        serde_json::json!({
            "graph": "inner",
            "sha256": sha256_hex(INNER.as_bytes()),
            "caps": [],
            "exit": "a",
            "input_keys": [],
            "parent": "docs",
        })
    );
}
