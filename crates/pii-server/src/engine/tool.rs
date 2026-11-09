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
    /// Spec 399 R8: the programs a `run` node may exec, a literal array or a `$.input.<name>` reference.
    programs: serde_json::Value,
    toolset: ToolSet,
}

/// The evaluation context a node's `$.` expressions resolve against (the same object `ToolNode` builds).
fn eval_ctx(input: &NodeInput) -> serde_json::Value {
    serde_json::json!({
        "input": input.input,
        "payload": input.payload,
        "nodes": input.nodes,
    })
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
        let programs = cfg
            .get("programs")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        Ok(Box::new(ToolNode {
            tool,
            args,
            programs,
            toolset: self.toolset.clone(),
        }))
    }

    /// Spec 399 R9: a `run` node's declared programs join its process scope (see `dispatch`).
    fn declared_programs(
        &self,
        cfg: &serde_json::Value,
        run_input: &serde_json::Value,
    ) -> Vec<String> {
        if cfg.get("tool").and_then(serde_json::Value::as_str) != Some("run") {
            return Vec::new();
        }
        let programs = cfg
            .get("programs")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        declared(&serde_json::json!({ "input": run_input }), &programs).unwrap_or_default()
    }
}

/// The program names a `run` node declares, resolved. Anything that is not an array of strings declares nothing,
/// so a malformed `programs` fails closed.
fn declared(ec: &serde_json::Value, programs: &serde_json::Value) -> Option<Vec<String>> {
    let resolved = eval_or_literal(ec, programs).ok()?;
    resolved
        .as_array()?
        .iter()
        .map(|p| p.as_str().map(str::to_owned))
        .collect()
}

impl ToolNode {
    /// Spec 399 R8: a `run` node execs an argv only when its first entry EQUALS a declared program. This is checked
    /// before the tool and before the process capability, so a program the run scope admits for another reason is
    /// still denied, and the refusal names it.
    fn check_declared(
        &self,
        ec: &serde_json::Value,
        resolved: &serde_json::Value,
    ) -> Result<(), PiiError> {
        if self.tool != "run" {
            return Ok(());
        }
        let program = resolved
            .get("argv")
            .and_then(serde_json::Value::as_array)
            .and_then(|argv| argv.first())
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if declared(ec, &self.programs)
            .unwrap_or_default()
            .iter()
            .any(|p| p == program)
        {
            Ok(())
        } else {
            Err(PiiError::Denied(format!(
                "program '{program}' is not in this node's `programs`"
            )))
        }
    }
}

impl GraphNode for ToolNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        // Resolve `$.`-expressions in args against the node's eval context.
        let ec = eval_ctx(input);
        let resolved = resolve_args(&ec, &self.args)?;
        let result = self.check_declared(&ec, &resolved).and_then(|()| {
            self.toolset.get(&self.tool).map_or_else(
                || Err(PiiError::Invalid(format!("unknown tool '{}'", self.tool))),
                |tool| tool.call(ctx, &resolved),
            )
        });
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

    /// Spec 108 R4: through the daemon's wiring (a tool set with NO observer,
    /// the run's observer a `ContractObserver` over the log) a bash deny-list
    /// hit emits `tools.deny_list.hit` and the call emits `tools.call`.
    #[test]
    fn a_deny_list_hit_reaches_the_log_as_tools_events() {
        let log = Arc::new(pii_observe::Recorder::new());
        let ctx = daemon_ctx(&log, CapSet::default());
        run_bash("sudo ls", &ctx);
        let hits = log.events_named(pii_observe::names::TOOLS_DENY_LIST_HIT);
        assert_eq!(hits.len(), 1, "{:?}", log.events());
        assert_eq!(
            hits[0].fields.get("entry"),
            Some(&serde_json::json!("sudo"))
        );
        let calls = log.events_named(pii_observe::names::TOOLS_CALL);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].fields.get("outcome"),
            Some(&serde_json::json!("denied"))
        );
    }

    /// Near-miss control: an allowed command logs `tools.call` (ok) and no hit.
    #[test]
    fn an_allowed_command_logs_a_call_and_no_deny_list_hit() {
        let log = Arc::new(pii_observe::Recorder::new());
        let caps = CapSet {
            kinds: pii_core::CapKind::PROC,
            proc_argv0: pii_core::ArgvScope::new(vec![String::from("/bin/sh")]),
            ..CapSet::default()
        };
        let ctx = daemon_ctx(&log, caps);
        run_bash("true", &ctx);
        assert_eq!(
            log.events_named(pii_observe::names::TOOLS_DENY_LIST_HIT),
            [] as [pii_observe::OwnedEvent; 0]
        );
        let calls = log.events_named(pii_observe::names::TOOLS_CALL);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].fields.get("outcome"),
            Some(&serde_json::json!("ok"))
        );
    }

    /// `ToolCallObserver` (the run chain's log tap) forwards tool calls and
    /// nothing else: a plain denial is not a tool call.
    #[test]
    fn the_tool_call_tap_forwards_calls_and_ignores_other_events() {
        let log = Arc::new(pii_observe::Recorder::new());
        let tap = pii_core::ToolCallObserver::new(log.clone());
        pii_core::Observer::event(
            &tap,
            &ObsEvent::Denial {
                principal: "p".into(),
                cap: "c".into(),
            },
        );
        assert_eq!(log.events(), [] as [pii_observe::OwnedEvent; 0]);
        let ctx = Ctx::with_observer(
            CapSet::default(),
            Budget::unlimited(),
            Arc::new(tap),
            trace(),
        );
        run_bash("sudo ls", &ctx);
        assert_eq!(
            log.events_named(pii_observe::names::TOOLS_DENY_LIST_HIT)
                .len(),
            1
        );
    }

    fn trace() -> TraceCtx {
        TraceCtx {
            run: pii_abi::RunId::from("r1"),
            span: 0,
        }
    }

    fn daemon_ctx(log: &Arc<pii_observe::Recorder>, caps: CapSet) -> Ctx {
        let obs = Arc::new(pii_core::ContractObserver::new(log.clone()));
        Ctx::with_observer(caps, Budget::unlimited(), obs, trace())
    }

    fn run_bash(command: &str, ctx: &Ctx) {
        let factory = ToolFactory {
            toolset: ToolSet::builtin(std::path::PathBuf::from(".")),
        };
        let node = factory
            .build(&serde_json::json!({"tool": "bash", "args": {"command": command}}))
            .unwrap();
        let input = NodeInput {
            input: serde_json::json!({}),
            payload: serde_json::json!({}),
            nodes: serde_json::json!({}),
        };
        node.run(ctx, &input).unwrap();
    }
}
