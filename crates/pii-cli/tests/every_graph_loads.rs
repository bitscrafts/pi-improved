//! Spec 380 R4: every top-level graph under `graphs/` loads through `pii graph lint`. A file the loader refuses is named in
//! the failure, so one bad file cannot hide behind the others.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn every_top_level_graph_lints_clean() {
    let dir = repo_root().join("graphs");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no graphs found under {}", dir.display());

    let mut refused = Vec::new();
    for file in &files {
        // Each run gets a fresh scratch directory: the lint store opens in the working directory.
        let scratch =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "cli-graph-lint-").unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_pii"))
            .current_dir(scratch.path())
            .env("PII_RUNTIME_DIR", scratch.path())
            .args(["graph", "lint", file.to_str().unwrap()])
            .output()
            .unwrap();
        if out.status.code() != Some(0) {
            refused.push(format!(
                "{} rc={:?}: {}{}",
                file.display(),
                out.status.code(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            ));
        }
    }
    assert!(
        refused.is_empty(),
        "graphs that do not load:\n{}",
        refused.join("\n")
    );
}

fn toml_files_under(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            toml_files_under(&path, files);
        } else if path.extension().is_some_and(|x| x == "toml") {
            files.push(path);
        }
    }
}

/// Spec 392 R7: the count comes from the parsed `[[node]]` tables, never from grep. Walks `graphs/` recursively, so the
/// starter graphs count too.
fn graphs_using_kind(kind: &str) -> Vec<String> {
    let mut files = Vec::new();
    toml_files_under(&repo_root().join("graphs"), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no graphs found under graphs/");

    let mut users = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let doc: toml::Table = toml::from_str(&text)
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
        let Some(nodes) = doc.get("node").and_then(toml::Value::as_array) else {
            continue;
        };
        let uses_kind = nodes.iter().any(|node| {
            node.as_table()
                .and_then(|node| node.get("kind"))
                .and_then(toml::Value::as_str)
                == Some(kind)
        });
        if uses_kind {
            users.push(file.display().to_string());
        }
    }
    users
}

#[test]
fn no_graph_uses_the_worker_kind() {
    let users = graphs_using_kind("worker");
    assert!(users.is_empty(), "graphs still using worker: {users:?}");
}

#[test]
fn no_graph_uses_the_classify_kind() {
    let users = graphs_using_kind("classify");
    assert!(users.is_empty(), "graphs still using classify: {users:?}");
}

#[test]
fn no_graph_uses_the_plan_kind() {
    let users = graphs_using_kind("plan");
    assert!(users.is_empty(), "graphs still using plan: {users:?}");
}

#[test]
fn no_graph_uses_the_memory_kind() {
    let users = graphs_using_kind("memory");
    assert!(users.is_empty(), "graphs still using memory: {users:?}");
}

#[test]
fn no_graph_uses_the_debug_crash_kind() {
    let users = graphs_using_kind("debug_crash");
    assert!(
        users.is_empty(),
        "graphs still using debug_crash: {users:?}"
    );
}

fn parse_graph(rel: &str) -> toml::Table {
    let text = std::fs::read_to_string(repo_root().join(rel)).unwrap();
    toml::from_str(&text).unwrap_or_else(|e| panic!("{rel} does not parse: {e}"))
}

fn node_kinds(graph: &toml::Table) -> Vec<&str> {
    graph
        .get("node")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|node| node.get("kind").and_then(toml::Value::as_str))
        .collect()
}

/// Spec 392 R7: a kind with one user is a graph written in Rust.
#[test]
fn every_kind_is_used_by_more_than_one_graph() {
    let mut files = Vec::new();
    toml_files_under(&repo_root().join("graphs"), &mut files);
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in &files {
        let graph: toml::Table = toml::from_str(&std::fs::read_to_string(file).unwrap())
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
        for kind in node_kinds(&graph) {
            users
                .entry(kind.to_owned())
                .or_default()
                .insert(file.display().to_string());
        }
    }
    assert!(users.len() > 1, "no kinds found under graphs/");
    let single: Vec<_> = users.iter().filter(|(_, u)| u.len() == 1).collect();
    assert!(single.is_empty(), "kinds with a single user: {single:?}");
}

/// Spec 392 R1: a worker node is a coder with a different brief.
#[test]
fn the_two_worker_graphs_load_as_coder() {
    for rel in ["graphs/code-index.toml", "graphs/intervention-report.toml"] {
        let graph = parse_graph(rel);
        assert!(
            node_kinds(&graph).contains(&"coder"),
            "{rel} has no coder node"
        );
        let out = Command::new(env!("CARGO_BIN_EXE_pii"))
            .args(["graph", "lint"])
            .arg(repo_root().join(rel))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{rel} does not lint: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Spec 392 R6: detection is a comparison in the graph, computed by a transform over the observation.
#[test]
#[allow(non_snake_case)]
fn CONTROL_the_detector_is_a_transform() {
    for rel in [
        "graphs/starter/heal-watch.toml",
        "graphs/starter/boss-watch.toml",
    ] {
        let graph = parse_graph(rel);
        assert!(
            !node_kinds(&graph).contains(&"heal_detect"),
            "{rel} still uses heal_detect"
        );
        let nodes = graph.get("node").and_then(toml::Value::as_array).unwrap();
        let detector = nodes
            .iter()
            .find(|node| node.get("id").and_then(toml::Value::as_str) == Some("detect"))
            .unwrap_or_else(|| panic!("{rel} has no detect node"));
        assert_eq!(
            detector.get("kind").and_then(toml::Value::as_str),
            Some("transform"),
            "{rel}: detect is not a transform"
        );
        let predicate = detector.get("expr").and_then(toml::Value::as_str).unwrap();
        assert!(
            predicate.contains("=="),
            "{rel}: detect computes no comparison: {predicate}"
        );
    }
}
