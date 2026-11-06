//! Spec 393 TDD Contract: `xtask lint-graph-kinds` fences the graph vocabulary. Driven through the real binary, the
//! way the gate runs it: with no argument it measures this repository, and `--root <dir>` points it at a scratch tree
//! holding `graphs/*.toml` and `graphs/VOCABULARY`. Every fixture copies the real `graphs/VOCABULARY`, so these tests
//! do not fix that file's column format.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The exit status and the combined output: a report on either stream counts, as `2>&1` in the Exit Criteria does.
fn lint(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("lint-graph-kinds")
        .args(args)
        .current_dir(root())
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// Lints a scratch tree of `(file stem, toml)` graphs under the shipped vocabulary.
fn lint_fixture(test: &str, graphs: &[(&str, &str)]) -> (bool, String) {
    let dir = root()
        .join("_tmp/lint-graph-kinds")
        .join(format!("{}-{test}", std::process::id()));
    let graph_dir = dir.join("graphs");
    std::fs::create_dir_all(&graph_dir).unwrap();
    std::fs::copy(
        root().join("graphs/VOCABULARY"),
        graph_dir.join("VOCABULARY"),
    )
    .expect("graphs/VOCABULARY declares the vocabulary (spec 393 R2)");
    for (stem, src) in graphs {
        std::fs::write(graph_dir.join(format!("{stem}.toml")), src).unwrap();
    }
    let got = lint(&["--root", dir.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).unwrap();
    got
}

fn transform_graph(name: &str, node: &str) -> String {
    format!(
        "name = \"{name}\"\nentry = \"{node}\"\n\n[[node]]\nid = \"{node}\"\nkind = \"transform\"\nexpr = \"$.input\"\n"
    )
}

#[test]
fn a_kind_outside_the_vocabulary_is_refused() {
    // The edge's `kind` is the trap the guardrail names: a grep would count `data` as a node kind.
    let graph = "name = \"outside\"\nentry = \"a\"\n\n\
                 [[node]]\nid = \"a\"\nkind = \"transform\"\nexpr = \"$.input\"\n\n\
                 [[node]]\nid = \"b\"\nkind = \"sel ect\"\n\n\
                 [[edge]]\nfrom = \"a\"\nto = \"b\"\nkind = \"data\"\n";
    let (ok, out) = lint_fixture("outside", &[("outside", graph)]);
    assert!(
        !ok,
        "a kind outside the vocabulary must fail the lint: {out}"
    );
    assert!(out.contains("sel ect"), "the refusal names the kind: {out}");
    assert!(
        !out.contains("data"),
        "an edge kind is not a node kind, so it is never reported: {out}"
    );
}

#[test]
fn a_single_user_kind_is_specialised() {
    // `route` is in the vocabulary, so the only thing wrong with this tree is that one graph alone uses it.
    let lone = "name = \"lone\"\nentry = \"pick\"\n\n\
                [[node]]\nid = \"pick\"\nkind = \"route\"\non = \"$.input.x\"\n\
                rules = [{ port = \"any\" }]\n";
    let first = transform_graph("first", "one");
    let second = format!(
        "{}\n[[node]]\nid = \"three\"\nkind = \"transform\"\nexpr = \"$.input\"\n",
        transform_graph("second", "two")
    );
    let (ok, out) = lint_fixture(
        "specialised",
        &[("first", &first), ("second", &second), ("lone", lone)],
    );
    assert!(!ok, "a single-user kind must fail the lint: {out}");
    assert!(
        out.contains("specialised") && !out.contains(" 0 specialised"),
        "the report says `specialised`, with a count above zero: {out}"
    );
    assert!(out.contains("route"), "the report names the kind: {out}");
}

#[test]
fn a_duplicate_flow_is_named() {
    let queue = std::fs::read_to_string(root().join("graphs/queue.toml")).unwrap();
    let copy = queue.replacen("name = \"queue\"", "name = \"zz-copy\"", 1);
    assert_ne!(copy, queue, "the fixture must rename its copy of the queue");
    let (ok, out) = lint_fixture("duplicate", &[("queue", &queue), ("zz-copy", &copy)]);
    assert!(!ok, "a re-implemented flow must fail the lint: {out}");
    assert!(
        out.contains("duplicate-of queue"),
        "the report names the graph it duplicates: {out}"
    );
}

/// `<N> kinds over <M> graphs, 0 specialised`: the count that makes a pass evidence (R6).
fn measured_counts(out: &str) -> Option<(usize, usize)> {
    let line = out.lines().find(|l| l.contains("0 specialised"))?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words.iter().position(|w| *w == "kinds")?;
    if words.get(at + 1) != Some(&"over") || words.get(at + 3) != Some(&"graphs,") {
        return None;
    }
    Some((
        words.get(at - 1)?.parse().ok()?,
        words[at + 2].parse().ok()?,
    ))
}

#[test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
fn CONTROL_every_shipped_graph_passes() {
    let (ok, out) = lint(&[]);
    assert!(ok, "every shipped graph must pass the fence: {out}");
    let (kinds, graphs) =
        measured_counts(&out).unwrap_or_else(|| panic!("a pass reports what it measured: {out}"));
    assert!(kinds > 0, "a pass over no kinds proves nothing: {out}");
    let top_level = std::fs::read_dir(root().join("graphs"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "toml")
        })
        .count();
    assert!(
        graphs >= top_level,
        "the lint measured {graphs} graphs but graphs/ holds {top_level}: {out}"
    );
}
