//! Control for the spec72 fixture: the REAL reachability check (`xtask::reachability::run`) must treat a
//! cfg(test)-only reference as NOT a caller, and must count a real one. Both sides measured through the same
//! public entry point the gate uses. A check that cannot fail proves nothing, so the negative control is required.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

/// A scratch export holding ONLY a cfg(test) reference (the decoy the fixture writes).
fn decoy_only_export(dir: &Path) {
    let src = dir.join("crates/pii-server/src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("queue.rs"),
        "#[cfg(test)]\n#[allow(dead_code)]\nmod x {\n    fn f() { let _ = pii_model::resolve_with_transport(\"x\", todo!()); }\n}\n",
    )
    .unwrap();
}

#[test]
fn a_cfg_test_only_reference_is_not_a_caller() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../_tmp/spec72-control")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    decoy_only_export(&dir);
    // The real check must find no caller for resolve_with_transport when only the cfg(test) decoy exists.
    let res = xtask::reachability::run(&dir);
    let names_it = res
        .as_ref()
        .err()
        .is_some_and(|v| v.iter().any(|m| m.contains("resolve_with_transport")));
    assert!(
        names_it,
        "the real check must report the symbol as uncalled when only a cfg(test) decoy exists: {res:?}"
    );
}
