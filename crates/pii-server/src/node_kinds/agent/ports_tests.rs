//! The agent factory's port list (spec 40 R1). Moved out of `tests.rs`, which sits at its 758-line cap (spec 424 R7).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(test)]

use std::sync::Arc;

use pii_abi::PortName;
use pii_graph::node::NodeFactory as _;

use super::AgentFactory;

/// R1 (spec 40): the agent factory advertises the `fail` port after
/// `ok`/`err`/`budget` (appended last, order-preserving for the existing
/// positional fixtures).
#[test]
fn agent_factory_ports_include_fail() {
    let ports = AgentFactory {
        build_provider: Arc::new(pii_model::provider_from_spec),
        provider: Arc::new(pii_model::ScriptedProvider::empty()),
        route_policy: std::collections::BTreeMap::new(),
        tiers: None,
        models: Arc::default(),
        workspace: std::path::PathBuf::from("."),
        live: None,
        memory: None,
    }
    .ports(&serde_json::json!({}))
    .expect("ports");
    assert_eq!(
        ports.iter().map(PortName::as_str).collect::<Vec<_>>(),
        vec!["ok", "err", "budget", "fail"],
        "fail appended last, ok/err/budget order preserved"
    );
}
