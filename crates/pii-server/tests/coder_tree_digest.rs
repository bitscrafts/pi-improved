//! The coder journals a digest of the workdir before and after the agent runs, so a run that changed nothing (`noop`)
//! is distinguishable from one that wrote a file (`write`): in the matrix both were `rc=0`, no output, node
//! `succeeded`, and the journal could not tell them apart. The digest covers path, size and mtime of the files in the
//! workdir; build output, VCS data and pii's own scratch are skipped, because the agent running a build must not look
//! like an edit. Controls: each skipped directory must really be skipped, and a real edit anywhere else must show.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pii_server::node_kinds::nodes::coder::tree::digest;

fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../_tmp")
        .join(format!("tree-digest-{name}-{}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn put(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn an_untouched_tree_has_the_same_digest() {
    let root = scratch("same");
    put(&root, "src/a.rs", "fn a() {}");
    assert_eq!(digest(&root).hash, digest(&root).hash);
}

#[test]
fn a_new_file_changes_the_digest() {
    let root = scratch("new");
    put(&root, "src/a.rs", "fn a() {}");
    let before = digest(&root);
    put(&root, "src/b.rs", "fn b() {}");
    let after = digest(&root);
    assert_ne!(before.hash, after.hash);
    assert_eq!(after.files, before.files + 1);
}

#[test]
fn an_edit_to_an_existing_file_changes_the_digest() {
    let root = scratch("edit");
    put(&root, "src/a.rs", "fn a() {}");
    let before = digest(&root);
    std::thread::sleep(Duration::from_millis(20));
    put(&root, "src/a.rs", "fn a() { 1; }");
    assert_ne!(before.hash, digest(&root).hash);
}

#[test]
fn a_deleted_file_changes_the_digest() {
    let root = scratch("del");
    put(&root, "src/a.rs", "fn a() {}");
    put(&root, "src/b.rs", "fn b() {}");
    let before = digest(&root);
    fs::remove_file(root.join("src/b.rs")).unwrap();
    assert_ne!(before.hash, digest(&root).hash);
}

#[test]
fn build_output_vcs_data_and_pii_scratch_are_not_edits() {
    // Near-miss controls for the skip rule: each of these directories changes, the digest must not.
    let root = scratch("skip");
    put(&root, "src/a.rs", "fn a() {}");
    let before = digest(&root);
    for dir in ["target", ".git", "_tmp", ".pii-sessions", "node_modules"] {
        put(&root, &format!("{dir}/x"), "noise");
    }
    assert_eq!(
        before.hash,
        digest(&root).hash,
        "build output and scratch must not look like an agent edit"
    );
}

#[test]
fn a_directory_that_only_resembles_a_skipped_one_still_counts() {
    // `target` is skipped, `targets` is source. A prefix match would hide a real edit.
    let root = scratch("resemble");
    put(&root, "src/a.rs", "fn a() {}");
    let before = digest(&root);
    put(&root, "targets/x.rs", "fn x() {}");
    put(&root, "tmp/y.rs", "fn y() {}");
    assert_ne!(before.hash, digest(&root).hash);
}

#[test]
fn a_missing_workdir_digests_as_empty_instead_of_failing() {
    let d = digest(Path::new("/nonexistent/pii-tree-digest-test"));
    assert_eq!(d.files, 0);
}
