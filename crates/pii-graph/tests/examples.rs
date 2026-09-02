//! Integration tests over the checked-in example graphs (spec F7).
//! Exercises `load_toml` + `validate` for every `examples/*.toml`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_graph::node::NodeRegistry;
use pii_graph::toml_load::load_toml;
use pii_graph::validate;

fn load(path: &str) -> pii_graph::spec::GraphSpec {
    // Tests run from the crate dir; examples live two levels up at the workspace root.
    let ws_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let full = ws_root.join("examples").join(path);
    let src =
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    load_toml(&src).unwrap_or_else(|e| panic!("load {}: {e}", full.display()))
}

#[test]
fn functional_examples_validate_clean() {
    let reg = NodeRegistry::builtin();
    for name in [
        "diamond.toml",
        "routed-diamond.toml",
        "race.toml",
        "crash-resume.toml",
        "emit-path.toml",
    ] {
        let spec = load(name);
        assert!(
            validate(&spec, &reg).is_ok(),
            "{name} should validate clean (warnings allowed)"
        );
    }
}

#[test]
fn lint_negatives_rejected() {
    let reg = NodeRegistry::builtin();
    let bad_port = load("lint-bad-port.toml");
    let errs = validate(&bad_port, &reg).unwrap_err();
    assert!(
        errs.iter().any(|e| e.starts_with("V1")),
        "want V1: {errs:?}"
    );

    let cap = load("lint-cap-escalation.toml");
    let errs = validate(&cap, &reg).unwrap_err();
    assert!(
        errs.iter().any(|e| e.starts_with("V2")),
        "want V2: {errs:?}"
    );

    let cycle = load("lint-unbounded-cycle.toml");
    let errs = validate(&cycle, &reg).unwrap_err();
    assert!(
        errs.iter().any(|e| e.starts_with("V3")),
        "want V3: {errs:?}"
    );

    let one_arm = load("lint-one-armed-race.toml");
    let errs = validate(&one_arm, &reg).unwrap_err();
    assert!(
        errs.iter().any(|e| e.starts_with("V9")),
        "want V9: {errs:?}"
    );
}
