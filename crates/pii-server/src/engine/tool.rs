//! `tool` node kind (F2.12): invokes one built-in tool with `$.`-resolved
//! args. Ports `ok`/`err`; a tool error is a routable `Next{err,..}`, not a
//! run abort.

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_graph::expr::eval_or_literal;
use pii_graph::node::GraphNode;
use pii_tools::ToolSet;

/// Tool node config (F2.12).
pub struct ToolNode {
    tool: String,
    args: serde_json::Value,
    toolset: ToolSet,
}

/// Factory registered into the engine's registry.
pub struct ToolFactory {
    /// Tool set all tool nodes share.
    pub toolset: ToolSet,
}

impl pii_graph::node::NodeFactory for ToolFactory {
    fn kind(&self) -> &'static str {
        "tool"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> pii_core::CapKind {
        pii_core::CapKind::FS_READ | pii_core::CapKind::FS_WRITE | pii_core::CapKind::PROC
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let tool = cfg
            .get("tool")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid("tool node requires 'tool' string".into()))?
            .to_owned();
        let args = cfg.get("args").cloned().unwrap_or(serde_json::Value::Null);
        Ok(Box::new(ToolNode {
            tool,
            args,
            toolset: self.toolset.clone(),
        }))
    }
}

impl GraphNode for ToolNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        // Resolve `$.`-expressions in args against the node's eval context.
        let ec = serde_json::json!({
            "input": input.input,
            "payload": input.payload,
            "nodes": input.nodes,
        });
        let resolved = resolve_args(&ec, &self.args)?;
        let result = self.toolset.get(&self.tool).map_or_else(
            || Err(PiiError::Invalid(format!("unknown tool '{}'", self.tool))),
            |tool| tool.call(ctx, &resolved),
        );
        self.toolset
            .observe_call(ctx, &self.tool, &resolved, &result);
        match result {
            Ok(v) => Ok(Outcome::Next {
                port: PortName::from("ok"),
                value: v,
            }),
            Err(e) => Ok(Outcome::Next {
                port: PortName::from("err"),
                value: serde_json::json!({ "error": e.to_string() }),
            }),
        }
    }
}

/// Recursively resolves `$.`-string leaves in `args` (F2.12).
fn resolve_args(
    ctx: &serde_json::Value,
    v: &serde_json::Value,
) -> Result<serde_json::Value, PiiError> {
    match v {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), resolve_args(ctx, val)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                out.push(resolve_args(ctx, item)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        other => eval_or_literal(ctx, other),
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

    use std::sync::Arc;

    use pii_core::{Budget, CapSet, Ctx, ObsEvent, RecordingObserver, TraceCtx};
    use pii_graph::node::NodeFactory;

    use super::*;

    fn run(command: &str, caps: CapSet) -> (Arc<RecordingObserver>, serde_json::Value) {
        let factory = ToolFactory {
            toolset: ToolSet::builtin(std::path::PathBuf::from(".")),
        };
        let node = factory
            .build(&serde_json::json!({"tool": "bash", "args": {"command": command}}))
            .unwrap();
        let rec = Arc::new(RecordingObserver::new());
        let ctx = Ctx::with_observer(
            caps,
            Budget::unlimited(),
            rec.clone(),
            TraceCtx {
                run: pii_abi::RunId::from("r1"),
                span: 0,
            },
        );
        let input = NodeInput {
            input: serde_json::json!({}),
            payload: serde_json::json!({}),
            nodes: serde_json::json!({}),
        };
        let out = node.run(&ctx, &input).unwrap();
        let Outcome::Next { value, .. } = out else {
            panic!("expected Next");
        };
        (rec, value)
    }

    /// Spec 107 R3: a tool-level denial (bash deny list) is reported through
    /// the run's observer as a denial, so it can reach the audit trail.
    #[test]
    fn a_denied_tool_call_is_reported_as_a_denial() {
        let (rec, value) = run("sudo ls", CapSet::default());
        assert!(value["error"].as_str().unwrap().contains("deny list"));
        let denials: Vec<_> = rec
            .events()
            .into_iter()
            .filter_map(|e| match e {
                ObsEvent::Denial { cap, .. } => Some(cap),
                _ => None,
            })
            .collect();
        assert_eq!(denials.len(), 1, "{denials:?}");
        assert!(denials[0].starts_with("tool:bash"), "{denials:?}");
    }

    /// Near-miss control: an allowed call reports no denial.
    #[test]
    fn a_call_that_is_not_denied_reports_no_denial() {
        let caps = CapSet {
            kinds: pii_core::CapKind::PROC,
            proc_argv0: pii_core::ArgvScope::new(vec![String::from("/bin/sh")]),
            ..CapSet::default()
        };
        let (rec, value) = run("true", caps);
        assert!(value.get("error").is_none(), "{value}");
        let denied = rec
            .events()
            .into_iter()
            .any(|e| matches!(e, ObsEvent::Denial { .. }));
        assert!(!denied);
    }
}
