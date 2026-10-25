//! Spec 267: the `require` node kind.
//!
//! A call whose sub-graph declares `needs` gets one, ahead of the sub-graph entry, that refuses the run when a declared
//! key resolves to nothing, before any sub-graph node runs.

use crate::expr::eval;
use crate::node::{GraphNode, NodeFactory};
use crate::nodes::eval_ctx;
use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};

/// `require`: cfg `{ call, graph, keys = { <name> = "<expr>", .. } }`. `call` and `graph` only name the refusal.
///
/// Evaluates every key's expression in table order. The first that is `Null` (or does not resolve) ends the run with
/// `graph.call.need`; otherwise the node passes its input on, unchanged, on `ok`. An empty string or `false` is a value.
pub struct RequireNode {
    call: String,
    graph: String,
    keys: Vec<(String, String)>,
}

impl RequireNode {
    /// Builds from cfg.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when `keys` is absent, is not a table, or holds an expression that is not a string.
    pub fn new(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let table = cfg
            .get("keys")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                PiiError::Invalid("require requires 'keys' (a table of name = expression)".into())
            })?;
        let keys = table
            .iter()
            .map(|(name, expr)| {
                expr.as_str()
                    .map(|e| (name.clone(), e.to_string()))
                    .ok_or_else(|| {
                        PiiError::Invalid(format!("require key '{name}' needs a string expression"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let field = |key: &str| {
            cfg.get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        Ok(Self {
            call: field("call"),
            graph: field("graph"),
            keys,
        })
    }
}

impl GraphNode for RequireNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let ec = eval_ctx(input);
        for (name, expr) in &self.keys {
            if matches!(eval(&ec, expr), Ok(v) if !v.is_null()) {
                continue;
            }
            return Ok(Outcome::Fail {
                error: format!(
                    "graph.call.need: call '{}' graph '{}' needs '{name}'",
                    self.call, self.graph
                ),
            });
        }
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: input.payload.clone(),
        })
    }
}

/// Built-in factory for the `require` node kind.
pub struct RequireFactory;

impl NodeFactory for RequireFactory {
    fn kind(&self) -> &'static str {
        "require"
    }

    fn effects(&self) -> Effects {
        Effects::PURE
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(RequireNode::new(cfg)?))
    }
}
