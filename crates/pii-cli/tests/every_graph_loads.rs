//! Spec 380 R4: every top-level graph under `graphs/` loads through `pii graph lint`. A file the loader refuses is named in
//! the failure, so one bad file cannot hide behind the others.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its stdout.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

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
        "graphs that do not load:\n{}",
        refused.join("\n")
    );
}
