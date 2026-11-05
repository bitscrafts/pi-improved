//! Spec 384: the installation ships a closed, minimal set of graphs. `graphs/minimal.txt` is the decision; these tests
//! hold it to three things: every named graph lints, every `call` target of a named graph is named too, and every pin
//! matches the file on disk.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The manifest's names, in file order, with comments and blank lines dropped.
fn manifest() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join("graphs/minimal.txt"))
        .expect("graphs/minimal.txt must exist");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn graph_path(name: &str) -> PathBuf {
    repo_root().join("graphs").join(format!("{name}.toml"))
}

fn sha256_hex(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    let mut hex = String::new();
    for b in Sha256::digest(&bytes) {
        write!(hex, "{b:02x}").unwrap();
    }
    hex
}

#[test]
fn manifest_names_real_unique_graphs() {
    let names = manifest();
    assert!(!names.is_empty(), "graphs/minimal.txt names no graphs");
    let unique: BTreeSet<&String> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "graphs/minimal.txt names a graph twice"
    );
    for name in &names {
        assert!(
            graph_path(name).is_file(),
            "graphs/minimal.txt names '{name}' but {} is missing",
            graph_path(name).display()
        );
    }
}

#[test]
fn every_manifest_graph_lints() {
    let mut refused = Vec::new();
    for name in manifest() {
        let file = graph_path(&name);
        // The lint store opens in the working directory: a scratch one keeps the repository clean.
        let scratch = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
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
        "manifest graphs that do not lint:\n{}",
        refused.join("\n")
    );
}

#[test]
fn manifest_is_closed_and_every_pin_matches() {
    let names = manifest();
    let set: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let mut problems = Vec::new();
    for name in &names {
        let text = std::fs::read_to_string(graph_path(name)).unwrap();
        let doc: toml::Table = toml::from_str(&text).unwrap();
        let Some(nodes) = doc.get("node").and_then(toml::Value::as_array) else {
            continue;
        };
        for node in nodes {
            let Some(node) = node.as_table() else {
                continue;
            };
            if node.get("kind").and_then(toml::Value::as_str) != Some("call") {
                continue;
            }
            let target = node
                .get("graph")
                .and_then(toml::Value::as_str)
                .unwrap_or("");
            if !set.contains(target) {
                problems.push(format!(
                    "{name} calls '{target}', which the manifest does not name"
                ));
                continue;
            }
            let pin = node
                .get("sha256")
                .and_then(toml::Value::as_str)
                .unwrap_or("");
            let actual = sha256_hex(&graph_path(target));
            if pin != actual {
                problems.push(format!(
                    "{name} pins '{target}' at {pin}, but the file is {actual}"
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "the manifest is not closed:\n{}",
        problems.join("\n")
    );
}
