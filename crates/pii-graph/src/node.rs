//! The harness ABI and node registry (spec F4.4).

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
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
}

/// A placeholder whose `run` is replaced once the scheduler pass lands.
struct StubNode;
impl GraphNode for StubNode {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        Err(PiiError::Invalid(
            "node runtime is not implemented in this phase (scheduler pass)".into(),
        ))
    }
}

macro_rules! impl_factory {
    ($t:ident, $kind:literal, $fx:expr, $req:expr, $default_ports:expr) => {
        #[doc = concat!("Built-in factory for the `", $kind, "` node kind.")]
        #[allow(non_camel_case_types)]
        pub struct $t;
        impl NodeFactory for $t {
            fn kind(&self) -> &'static str {
                $kind
            }
            fn effects(&self) -> Effects {
                $fx
            }
            fn required_caps(&self) -> CapKind {
                $req
            }
            fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
                Ok($default_ports)
            }
            fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
                Ok(Box::new(StubNode))
            }
        }
    };
}

impl_factory!(
    TransformFactory,
    "transform",
    Effects::Pure,
    CapKind::empty(),
    vec![PortName::from("ok"), PortName::from("err")]
);

impl_factory!(
    WaitFactory,
    "wait",
    Effects::Pure,
    CapKind::empty(),
    vec![PortName::from("fired"), PortName::from("timeout")]
);

impl_factory!(
    EmitFactory,
    "emit",
    Effects::WritesFs,
    CapKind::FS_WRITE,
    vec![PortName::from("ok"), PortName::from("err")]
);

/// `route` ports are data-defined by `rules[].port`.
#[allow(non_camel_case_types)]
#[doc = "Built-in factory for the `route` node kind."]
pub struct RouteFactory;
impl NodeFactory for RouteFactory {
    fn kind(&self) -> &'static str {
        "route"
    }
    fn effects(&self) -> Effects {
        Effects::Pure
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
    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(StubNode))
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

    use super::NodeRegistry;
    use pii_abi::Effects;
    use pii_core::CapKind;

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
        assert_eq!(reg.factory("transform").unwrap().effects(), Effects::Pure);
        assert_eq!(reg.factory("emit").unwrap().effects(), Effects::WritesFs);
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
