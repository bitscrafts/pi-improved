//! The harness ABI and node registry (spec F4.4).

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName, ResumeKey, RunEvent};
use pii_core::CapKind;
use pii_core::Ctx;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A single node instance during a run.
pub trait GraphNode: Send + Sync {
    /// Executes the node against an input, returning an outcome.
    ///
    /// # Errors
    /// Returns [`PiiError`] on failure or cancellation.
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError>;

    /// Resumes a node that suspended under `key`, instead of re-running it
    /// from scratch (spec 30 D1). `Some(outcome)` completes the resume with
    /// that outcome (typically `Next`); `None` falls back to re-dispatching
    /// [`GraphNode::run`] (the pre-spec-30 behavior, still correct for a
    /// wait condition that has not yet been satisfied).
    fn on_resume(&self, _ctx: &Ctx, _input: &NodeInput, _key: &ResumeKey) -> Option<Outcome> {
        None
    }

    /// Extra journal events to append alongside this node's own
    /// `NodeFinished`, given the value it just produced on `Outcome::Next`
    /// (spec 32 R1: the `spawn` node's parent→child `RunSpawned` link).
    /// Appended through the SAME [`crate::journal::Journaler`] that owns
    /// this run's seq counter — never written by the node itself, which
    /// would race the journaler's write-behind buffer. Default: none.
    fn link_events(&self, _value: &serde_json::Value) -> Vec<RunEvent> {
        Vec::new()
    }
}

/// Builds node instances for one `kind`.
pub trait NodeFactory: Send + Sync {
    /// The stable kind string this factory handles.
    fn kind(&self) -> &'static str;
    /// The output ports this kind may take, given its config.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] if the config is malformed.
    fn ports(&self, cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError>;
    /// Static capability bits required to run this kind.
    fn required_caps(&self) -> CapKind;
    /// Static effect classification.
    fn effects(&self) -> Effects;
    /// Builds a node instance for a config.
    ///
    /// # Errors
    /// Returns [`PiiError::Invalid`] if the config is malformed.
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError>;
}

/// A registry of node factories keyed by kind.
#[derive(Default)]
pub struct NodeRegistry {
    map: BTreeMap<&'static str, Arc<dyn NodeFactory>>,
}

impl NodeRegistry {
    /// Builds the registry with the four built-in kinds.
    #[must_use]
    pub fn builtin() -> Self {
        let mut map: BTreeMap<&'static str, Arc<dyn NodeFactory>> = BTreeMap::new();
        map.insert("transform", Arc::new(TransformFactory));
        map.insert("route", Arc::new(RouteFactory));
        map.insert("emit", Arc::new(EmitFactory));
        map.insert("wait", Arc::new(WaitFactory));
        map.insert("timer", Arc::new(TimerFactory));
        Self { map }
    }

    /// Looks up a factory by kind.
    ///
    /// # Errors
    /// Returns [`PiiError::NotFound`] if the kind is unknown.
    pub fn factory(&self, kind: &str) -> Result<&Arc<dyn NodeFactory>, PiiError> {
        self.map
            .get(kind)
            .ok_or_else(|| PiiError::NotFound(format!("unknown node kind '{kind}'")))
    }

    /// Registers an additional factory (custom kinds; tests, plugins).
    pub fn register(&mut self, factory: Arc<dyn NodeFactory>) {
        let kind = factory.kind();
        self.map.insert(kind, factory);
    }

    /// The kind's `required_caps()`; empty for an unknown kind. Unknown kinds
    /// are rejected by `validate()` — the default must not invent capabilities.
    #[must_use]
    pub fn required_caps_of(&self, kind: &str) -> CapKind {
        self.factory(kind)
            .map_or_else(|_| CapKind::empty(), |f| f.required_caps())
    }
}

#[doc = "Built-in factory for the `transform` node kind."]
#[allow(non_camel_case_types)]
pub struct TransformFactory;
#[doc = "Built-in factory for the `wait` node kind."]
#[allow(non_camel_case_types)]
pub struct WaitFactory;
#[doc = "Built-in factory for the `emit` node kind."]
#[allow(non_camel_case_types)]
pub struct EmitFactory;

impl NodeFactory for TransformFactory {
    fn kind(&self) -> &'static str {
        "transform"
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(crate::nodes::TransformNode::new(cfg)?))
    }
}

impl NodeFactory for WaitFactory {
    fn kind(&self) -> &'static str {
        "wait"
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("fired"), PortName::from("timeout")])
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(crate::nodes::WaitNode::new(cfg)?))
    }
}

impl NodeFactory for EmitFactory {
    fn kind(&self) -> &'static str {
        "emit"
    }
    fn effects(&self) -> Effects {
        Effects::WRITES_FS
    }
    fn required_caps(&self) -> CapKind {
        CapKind::FS_WRITE
    }
    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(crate::nodes::EmitNode::new(cfg)?))
    }
}

/// `route` ports are data-defined by `rules[].port`.
#[allow(non_camel_case_types)]
#[doc = "Built-in factory for the `route` node kind."]
pub struct RouteFactory;
impl NodeFactory for RouteFactory {
    fn kind(&self) -> &'static str {
        "route"
    }
    fn effects(&self) -> Effects {
        Effects::PURE
    }
    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }
    fn ports(&self, cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        let rules = cfg
            .get("rules")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PiiError::Invalid("route requires a 'rules' array".into()))?;
        let mut ports = Vec::new();
        for r in rules {
            let p = r
                .get("port")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PiiError::Invalid("route rule missing 'port' string".into()))?;
            let pn = PortName::from(p);
            if !ports.contains(&pn) {
                ports.push(pn);
            }
        }
        if ports.is_empty() {
            return Err(PiiError::Invalid(
                "route 'rules' must define at least one port".into(),
            ));
        }
        Ok(ports)
    }
    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(crate::nodes::RouteNode::new(cfg)?))
    }
}
/// Built-in factory for the `timer` node kind (F5.6).
#[allow(non_camel_case_types)]
#[doc = "Built-in factory for the `timer` node kind."]
pub struct TimerFactory;

impl NodeFactory for TimerFactory {
    fn kind(&self) -> &'static str {
        "timer"
    }

    fn effects(&self) -> Effects {
        Effects::PURE
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("fired"), PortName::from("timeout")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(crate::nodes::TimerNode::new(cfg)?))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::nodes::{RouteNode, TransformNode};
    use pii_abi::NodeInput;
    use pii_core::{CapKind, CapSet, Ctx};

    fn ctx() -> Ctx {
        Ctx::root(CapSet::empty(), pii_core::Budget::unlimited())
    }

    fn test_input() -> NodeInput {
        NodeInput {
            input: serde_json::json!({"branch": "a", "n": 3}),
            payload: serde_json::json!({"x": 1}),
            nodes: serde_json::json!({}),
        }
    }

    #[test]
    fn transform_expr_evaluates() {
        let n = TransformNode::new(&serde_json::json!({"expr": "$.input.n"})).unwrap();
        match n.run(&ctx(), &test_input()).unwrap() {
            Outcome::Next { port, value } => {
                assert_eq!(port.as_str(), "ok");
                assert_eq!(value, serde_json::json!(3));
            }
            _ => panic!("expected Next"),
        }
    }

    #[test]
    fn transform_literal_passthrough() {
        let n = TransformNode::new(&serde_json::json!({"value": 42})).unwrap();
        let o = n.run(&ctx(), &test_input()).unwrap();
        assert!(matches!(
            o,
            Outcome::Next { ref value, .. } if value == &serde_json::json!(42)
        ));
    }

    #[test]
    fn route_picks_matching_rule() {
        let n = RouteNode::new(&serde_json::json!({
            "on": "$.input.branch",
            "rules": [{"eq": "a", "port": "a"}, {"port": "b"}]
        }))
        .unwrap();
        match n.run(&ctx(), &test_input()).unwrap() {
            Outcome::Next { port, .. } => assert_eq!(port.as_str(), "a"),
            _ => panic!("expected Next"),
        }
    }

    #[test]
    fn route_falls_back_to_portless_rule() {
        let n = RouteNode::new(&serde_json::json!({
            "on": "$.input.branch",
            "rules": [{"eq": "zzz", "port": "a"}, {"port": "b"}]
        }))
        .unwrap();
        match n.run(&ctx(), &test_input()).unwrap() {
            Outcome::Next { port, .. } => assert_eq!(port.as_str(), "b"),
            _ => panic!("expected Next"),
        }
    }

    #[test]
    fn factory_build_returns_working_nodes() {
        let reg = NodeRegistry::builtin();
        let node = reg
            .factory("transform")
            .unwrap()
            .build(&serde_json::json!({"expr": "$.input.n"}))
            .unwrap();
        match node.run(&ctx(), &test_input()).unwrap() {
            Outcome::Next { value, .. } => assert_eq!(value, serde_json::json!(3)),
            _ => panic!("expected Next"),
        }
    }

    #[test]
    fn builtin_registry_exposes_four_kinds() {
        let reg = NodeRegistry::builtin();
        assert!(reg.factory("transform").is_ok());
        assert!(reg.factory("route").is_ok());
        assert!(reg.factory("emit").is_ok());
        assert!(reg.factory("wait").is_ok());
        assert!(reg.factory("bogus").is_err());
    }

    #[test]
    fn factories_report_effects_and_caps() {
        let reg = NodeRegistry::builtin();
        assert_eq!(reg.factory("transform").unwrap().effects(), Effects::PURE);
        assert_eq!(reg.factory("emit").unwrap().effects(), Effects::WRITES_FS);
        assert_eq!(
            reg.factory("emit").unwrap().required_caps(),
            CapKind::FS_WRITE
        );
        assert_eq!(
            reg.factory("wait").unwrap().required_caps(),
            CapKind::empty()
        );
    }

    #[test]
    fn route_ports_from_rules() {
        let reg = NodeRegistry::builtin();
        let cfg = serde_json::json!({"rules": [{"eq":"a","port":"a"},{"port":"b"}]});
        let ports = reg.factory("route").unwrap().ports(&cfg).unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].as_str(), "a");
        assert_eq!(ports[1].as_str(), "b");
    }
}
