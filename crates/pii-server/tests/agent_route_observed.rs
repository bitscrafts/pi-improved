//! Spec 185 (observability): the run reports the provider and model its agent ACTUALLY used. Read from
//! the agent's `--mode json` events, so a fallback to another route is visible in the journal.
//! Controls: a text run (no JSON) reports no route, and a fallback is reported as a distinct route.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::agent_worker::reported_routes;

#[test]
fn a_run_on_ollama_reports_ollama_only() {
    // The real session header line, as `pi --mode json` printed it on 2026-10-21 (ids and time trimmed).
    let out = concat!(
        r#"{"type":"session","version":3,"provider":"ollama-cloud","modelId":"deepseek-v4.1-flash:cloud","thinkingLevel":"off"}"#,
        "\n",
        r#"{"type":"message_end"}"#,
        "\n",
    );
    assert_eq!(
        reported_routes(out),
        vec![(
            String::from("ollama-cloud"),
            String::from("deepseek-v4.1-flash:cloud")
        )]
    );
}

#[test]
fn a_fallback_to_another_route_is_reported_as_a_second_route() {
    let out = concat!(
        r#"{"type":"session","provider":"ollama-cloud","modelId":"deepseek-v4.1-flash:cloud"}"#,
        "\n",
        r#"{"type":"session","provider":"anthropic","modelId":"claude-opus-5"}"#,
        "\n",
    );
    let routes = reported_routes(out);
    assert_eq!(routes.len(), 2, "the fallback must be visible, not hidden");
    assert!(routes.iter().any(|(p, _)| p == "anthropic"));
}

#[test]
fn a_text_run_reports_no_route_so_it_is_unknown_not_a_match() {
    assert!(reported_routes("PONG\n").is_empty());
    assert!(reported_routes("").is_empty());
}
