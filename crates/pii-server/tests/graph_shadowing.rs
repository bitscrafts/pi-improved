//! Spec 384: a system graph never resolves to a user graph.
//!
//! Derivation 047's first enforcement layer: the search path decides the dependency direction. A user
//! file named like a system graph would otherwise SHADOW it under first-match, and every system graph
//! calling `docs` by name would run user code — the one-way dependency defeated by a filename.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use pii_server::graph_paths::find_graph;

/// Scratch is rooted under the repo (`lint-scratch` refuses `temp_dir()`): the guard removes the
/// directory on drop, so a failed assertion cannot leave a tree behind either.
fn dir(tag: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), tag).unwrap()
}

fn write(p: &Path, name: &str) {
    std::fs::write(
        p.join(format!("{name}.toml")),
        format!("name = \"{name}\"\n"),
    )
    .unwrap();
}

#[test]
fn a_user_file_cannot_shadow_a_system_graph() {
    let system = dir("shadow-sys-");
    let user = dir("shadow-usr-");
    write(system.path(), "docs");
    write(user.path(), "docs");
    let dirs: Vec<PathBuf> = vec![system.path().to_path_buf(), user.path().to_path_buf()];
    let found = find_graph(&dirs, "docs");
    assert!(
        found.is_none(),
        "a name in two search dirs must be refused, not first-matched: {found:?}"
    );
}

#[test]
fn a_user_graph_with_its_own_name_resolves() {
    let system = dir("shadow-sys2-");
    let user = dir("shadow-usr2-");
    write(system.path(), "docs");
    write(user.path(), "my-night");
    let dirs: Vec<PathBuf> = vec![system.path().to_path_buf(), user.path().to_path_buf()];
    let found = find_graph(&dirs, "my-night");
    let want = user.path().join("my-night.toml");
    assert_eq!(
        found.as_deref(),
        Some(want.as_path()),
        "a user graph resolves"
    );
}

#[test]
fn control_a_single_copy_still_resolves() {
    let system = dir("shadow-sys3-");
    write(system.path(), "docs");
    let dirs: Vec<PathBuf> = vec![system.path().to_path_buf()];
    let found = find_graph(&dirs, "docs");
    let want = system.path().join("docs.toml");
    assert_eq!(
        found.as_deref(),
        Some(want.as_path()),
        "one copy is not a shadow"
    );
}
