//! Spec 136 TDD Contract — controls for the `lint-scratch` enforcement.

use xtask::lint_scratch::check_source;

/// Built at runtime so this file does not trip the lint it tests.
/// A `_tmp` directory literal, kept out of the fixture fn.
const BARE_DIR: &str = "/p/_tmp";
const NEEDLE: &str = concat!("temp_", "dir()");

#[test]
fn temp_dir_is_refused() {
    let src = format!("fn t() {{\n    let d = std::env::{NEEDLE}.join(\"x\");\n}}\n");
    let got = check_source("crates/a/src/lib.rs", &src);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(
        got.first()
            .is_some_and(|m| m.starts_with("crates/a/src/lib.rs:2:")),
        "{got:?}"
    );
}

#[test]
fn imported_temp_dir_is_refused() {
    let src = format!("use std::env;\nfn t() {{\n    let d = env::{NEEDLE};\n}}\n");
    assert_eq!(check_source("a.rs", &src).len(), 1);
}

#[test]
fn tempdir_in_under_the_tree_still_passes() {
    let src = "fn t() {\n    let root = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"_tmp\");\n    std::fs::create_dir_all(&root).unwrap();\n    let d = tempfile::Builder::new().tempdir_in(&root).unwrap();\n}\n";
    assert_eq!(check_source("a.rs", src), [] as [String; 0]);
}

#[test]
fn comments_and_docs_naming_temp_dir_pass() {
    let src = format!(
        "// never call std::env::{NEEDLE} here\n/// not `env::{NEEDLE}` either\nfn t() {{}}\n"
    );
    assert_eq!(check_source("a.rs", &src), [] as [String; 0]);
}

#[test]
fn bare_tempdir_in_is_still_refused() {
    let src = format!("fn t() {{\n    b.tempdir_in(\"{BARE_DIR}\").unwrap();\n}}\n");
    assert_eq!(check_source("a.rs", &src).len(), 1);
}
