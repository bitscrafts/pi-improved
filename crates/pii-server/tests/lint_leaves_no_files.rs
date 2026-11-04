//! Spec 371 R1: `pii graph lint` leaves no `lint-*.redb` file behind, in the state dir or the runtime dir.
//! Lint opens the approval, memory, queue and heal stores only to know their node kinds, so those stores are
//! in-memory; the lint still checks the same kinds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

/// Every `lint-*.redb` file under `root`, at any depth.
fn lint_files_under(root: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            lint_files_under(&path, found);
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("lint-"))
            && path.extension().is_some_and(|e| e == "redb")
        {
            found.push(path);
        }
    }
}

#[test]
fn a_lint_leaves_no_redb_file_in_the_state_or_runtime_dir() {
    let state = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    let reg = pii_server::lint_registry(state.path(), runtime.path());
    // The four store-backed kinds must still be known to lint, so lint still checks them.
    for kind in ["gate", "memory", "select", "heal_detect"] {
        assert!(reg.factory(kind).is_ok(), "lint must still know '{kind}'");
    }

    let mut found = Vec::new();
    lint_files_under(state.path(), &mut found);
    lint_files_under(runtime.path(), &mut found);
    assert!(found.is_empty(), "lint left files behind: {found:?}");
}
