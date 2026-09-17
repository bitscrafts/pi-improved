//! Spec-29 R6: `max_tool_calls` lives on `GraphLimits` and parses from
//! `[limits]` (the run's `Budget` is built from it in `pii-server/runs.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// R6: `max_tool_calls` is parsed from the graph's `[limits]` table.
#[test]
fn max_tool_calls_in_limits() {
    let toml = r#"
name = "t"
entry = "a"

[limits]
max_tool_calls = 2

[[node]]
id = "a"
kind = "transform"
value = "1"
"#;
    let spec = pii_graph::toml_load::load_toml(toml).expect("parses");
    assert_eq!(spec.limits.max_tool_calls, 2);
    // Default: no behavior change (N2 — u64::MAX means unlimited).
    let default_spec = pii_graph::toml_load::load_toml(
        r#"
name = "t"
entry = "a"

[[node]]
id = "a"
kind = "transform"
value = "1"
"#,
    )
    .expect("parses");
    assert_eq!(default_spec.limits.max_tool_calls, u64::MAX);
}
