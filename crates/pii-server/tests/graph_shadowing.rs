//! Spec 384: the search path resolves by PRECEDENCE, and the rule is stated rather than implied.
//!
//! Derivation 047's first enforcement layer is the search path: a user graph may call a system graph,
//! never the reverse. That is a property of which dir is searched first. Refusing duplicate names at
//! LOAD was tried and reverted — a host legitimately holds the same names in its installation and in its
//! devbox copy, and refusing them made `spec` unable to reach `docs` and `preflight` (measured
//! 2026-11-05: every dispatch aborted with `graph.call.not_found`). The refusal that survives is at the
//! INSTALLER (never write a user graph over a system name) and in the CLI lint (report the shadow).
//!
//! These tests pin the precedence, so a later change cannot silently invert it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: a test fixture builds scratch dirs; a failure there is the test failing, not a defect.

use pii_server::find_graph;
use std::path::PathBuf;

fn dir(name: &str, graphs: &[(&str, &str)]) -> PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("_tmp")
        .join(format!("shadowing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for (file, body) in graphs {
        std::fs::write(root.join(format!("{file}.toml")), body).unwrap();
    }
    root
}

#[test]
fn the_first_dir_wins_when_a_name_exists_in_both() {
    let system = dir(
        "system",
        &[("docs", "name = \"docs\""), ("night", "name = \"night\"")],
    );
    let user = dir("user", &[("docs", "name = \"docs\" # the user's copy")]);
    let dirs = vec![system.clone(), user];
    let got = find_graph(&dirs, "docs").expect("docs resolves");
    assert_eq!(
        got,
        system.join("docs.toml"),
        "the INSTALLATION is searched first: a user copy cannot take the name"
    );
}

#[test]
fn a_user_graph_with_its_own_name_resolves() {
    let system = dir("sys2", &[("docs", "name = \"docs\"")]);
    let user = dir("user2", &[("my-night", "name = \"my-night\"")]);
    let dirs = vec![system, user.clone()];
    assert_eq!(
        find_graph(&dirs, "my-night").expect("my-night resolves"),
        user.join("my-night.toml")
    );
}

// CONTROL for the first-dir rule: a name NOBODY has must still resolve to nothing.
#[test]
fn a_name_in_no_dir_resolves_to_nothing() {
    let system = dir("sys3", &[("docs", "name = \"docs\"")]);
    assert!(find_graph(&[system], "nowhere").is_none());
}

// CONTROL for the shadowing rule: ONE copy is not a shadow, so it must resolve normally.
#[test]
fn one_copy_is_not_ambiguous() {
    let only = dir("only", &[("docs", "name = \"docs\"")]);
    assert_eq!(
        find_graph(std::slice::from_ref(&only), "docs").expect("resolves"),
        only.join("docs.toml")
    );
}
