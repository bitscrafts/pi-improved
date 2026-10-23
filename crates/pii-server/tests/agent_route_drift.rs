//! A route the graph did not ask for must show up as a defect, not as a pass. pi can swap models mid-turn when
//! `retry.fallback_chains` is configured (v0.4.0 `failover.rs`: quota, overload and transient errors), and the swap
//! costs a different provider's quota. Two readers cover it: `failovers` reads pi's `failover_start` events, and
//! `route_drift` compares every reported route with the requested one.
//!
//! Provenance of the fixtures. The `session` and claude `init` lines are real captures (pi 0.2.0 on 2026-10-21 and a
//! haiku worker log). The `failover_start` line is NOT a capture: the installed pi is 0.2.0, which has no failover
//! (no such strings in the binary), so its shape is taken from the v0.4.0 source (`src/agent.rs` `AgentEvent::FailoverStart`,
//! tag `type`, `snake_case`; fields `camelCase`). It must be re-checked against a real stream once pi is upgraded.
//! Controls: a run on exactly the requested route reports no drift, and a text run is unknown, not a match.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_server::node_kinds::nodes::agent_route::{failovers, route_drift};
use pii_server::node_kinds::nodes::agent_worker::reported_routes;

const PI_SESSION: &str = r#"{"type":"session","version":3,"provider":"ollama-cloud","modelId":"deepseek-v4.1-flash:cloud","thinkingLevel":"off"}"#;
const FAILOVER: &str = r#"{"type":"failover_start","fromProvider":"ollama-cloud","fromModel":"deepseek-v4.1-flash:cloud","toProvider":"anthropic","toModel":"claude-opus-5","class":"quota","attempt":1}"#;
const CLAUDE_INIT: &str = r#"{"type":"system","subtype":"init","session_id":"9f8cea95-a50d-4cf3-bc54-12603578df48","model":"claude-haiku-5-5"}"#;

fn requested(p: &str, m: &str) -> (String, String) {
    (p.to_owned(), m.to_owned())
}

#[test]
fn a_failover_event_is_read_with_both_ends_and_the_class() {
    let f = failovers(&format!("{PI_SESSION}\n{FAILOVER}\n"));
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].from, "ollama-cloud/deepseek-v4.1-flash:cloud");
    assert_eq!(f[0].to, "anthropic/claude-opus-5");
    assert_eq!(f[0].class, "quota");
}

#[test]
fn a_run_with_no_failover_event_reports_none() {
    assert!(failovers(&format!("{PI_SESSION}\n")).is_empty());
}

#[test]
fn a_swapped_to_model_is_also_a_second_reported_route() {
    // After the swap pi names the new route on a later line; the route reader must keep both.
    let after = r#"{"type":"session","provider":"anthropic","modelId":"claude-opus-5"}"#;
    let routes = reported_routes(&format!("{PI_SESSION}\n{FAILOVER}\n{after}\n"));
    assert_eq!(routes.len(), 2);
}

#[test]
fn the_requested_route_alone_is_not_drift() {
    let routes = reported_routes(&format!("{PI_SESSION}\n"));
    assert!(
        route_drift(
            &requested("ollama-cloud", "deepseek-v4.1-flash:cloud"),
            &routes
        )
        .is_empty()
    );
}

#[test]
fn another_provider_is_drift() {
    let after = r#"{"type":"session","provider":"anthropic","modelId":"claude-opus-5"}"#;
    let routes = reported_routes(&format!("{PI_SESSION}\n{after}\n"));
    let drift = route_drift(
        &requested("ollama-cloud", "deepseek-v4.1-flash:cloud"),
        &routes,
    );
    assert_eq!(drift, vec![String::from("anthropic/claude-opus-5")]);
}

#[test]
fn the_same_provider_with_another_model_is_drift() {
    let other = r#"{"type":"session","provider":"ollama-cloud","modelId":"glm-5.3-flash:cloud"}"#;
    let routes = reported_routes(&format!("{other}\n"));
    let drift = route_drift(
        &requested("ollama-cloud", "deepseek-v4.1-flash:cloud"),
        &routes,
    );
    assert_eq!(
        drift,
        vec![String::from("ollama-cloud/glm-5.3-flash:cloud")]
    );
}

#[test]
fn a_run_that_reported_no_route_is_unknown_not_drift() {
    assert!(route_drift(&requested("ollama-cloud", "x"), &[]).is_empty());
}

#[test]
fn claude_asked_for_haiku_and_given_the_full_id_is_not_drift() {
    // The graph says `haiku`; claude reports `claude-haiku-5-5`. An alias contained in the id is the same route.
    let routes = reported_routes(&format!("{CLAUDE_INIT}\n"));
    assert!(route_drift(&requested("claude", "haiku"), &routes).is_empty());
}

#[test]
fn claude_asked_for_haiku_and_given_opus_is_drift() {
    let opus = r#"{"type":"system","subtype":"init","model":"claude-opus-5"}"#;
    let routes = reported_routes(&format!("{opus}\n"));
    assert_eq!(
        route_drift(&requested("claude", "haiku"), &routes),
        vec![String::from("claude/claude-opus-5")]
    );
}
