//! `sop` node kind (spec 113): the bookkeeping nodes the SOP compiler weaves
//! around each step.
//!
//! They run no work of their own (the step's agent / tool / gate / emit node
//! does). They check `requires`, name the step in the `sop.step.*` events, and
//! turn a failure into a run failure that names the SOP and the step.
//!
//! Ops: `begin` (check `requires`, emit started, hand an `act` step its
//! prompt), `await` (emit `awaiting_approval` for the gate that follows),
//! `end` (emit finished), `fail` (emit failed, fail the run).

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx, Principal};
use pii_observe::{Event, Level, Observer, RunId, names};

use pii_graph::node::{GraphNode, NodeFactory};

use super::gate::approval_id_for;

/// Registered into both registries; every `sop` node reports to one log.
pub struct SopFactory {
    /// Receives the `sop.step.*` events.
    pub log: Arc<dyn Observer>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Begin,
    Await,
    End,
    Fail,
}

impl NodeFactory for SopFactory {
    fn kind(&self) -> &'static str {
        "sop"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(match op_of(cfg)? {
            Op::Fail => Vec::new(),
            _ => vec![PortName::from("ok")],
        })
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let text = |k: &str| -> Result<String, PiiError> {
            cfg.get(k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| PiiError::Invalid(format!("sop node requires '{k}' (string)")))
        };
        let opt = |k: &str| {
            cfg.get(k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let requires = cfg
            .get("requires")
            .and_then(serde_json::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let op = op_of(cfg)?;
        let gate = opt("gate");
        if op == Op::Await && gate.is_none() {
            return Err(PiiError::Invalid(String::from(
                "sop await requires 'gate' (the gate node's id)",
            )));
        }
        Ok(Box::new(SopNode {
            op,
            sop: text("sop")?,
            step: text("step")?,
            kind: text("step_kind")?,
            requires,
            prompt: opt("prompt"),
            gate,
            cause: opt("cause"),
            log: Arc::clone(&self.log),
        }))
    }
}

fn op_of(cfg: &serde_json::Value) -> Result<Op, PiiError> {
    match cfg.get("op").and_then(serde_json::Value::as_str) {
        Some("begin") => Ok(Op::Begin),
        Some("await") => Ok(Op::Await),
        Some("end") => Ok(Op::End),
        Some("fail") => Ok(Op::Fail),
        other => Err(PiiError::Invalid(format!(
            "sop 'op' must be begin|await|end|fail, got {other:?}"
        ))),
    }
}

struct SopNode {
    op: Op,
    sop: String,
    step: String,
    kind: String,
    requires: Vec<String>,
    prompt: Option<String>,
    gate: Option<String>,
    cause: Option<String>,
    log: Arc<dyn Observer>,
}

impl SopNode {
    fn event(&self, ctx: &Ctx, level: Level, kind: &'static str) -> Event {
        Event::new("sop", level, kind)
            .field("sop", self.sop.clone())
            .field("step", self.step.clone())
            .field("kind", self.kind.clone())
            .in_run(RunId(run_of(ctx)))
    }

    /// Records the failure and returns the run-failing error naming the step.
    fn fail(&self, ctx: &Ctx, reason: &str) -> PiiError {
        self.log.event(
            &self
                .event(ctx, Level::Error, names::SOP_STEP_FAILED)
                .field("reason", reason.to_owned()),
        );
        PiiError::Invalid(format!(
            "sop '{}' step '{}' failed: {reason}",
            self.sop, self.step
        ))
    }
}

/// The run this node belongs to, read the way the `gate` reads it so the
/// approval id computed here is the id the gate will mint.
fn run_of(ctx: &Ctx) -> String {
    match ctx.principal() {
        Principal::GraphNode { run, .. } => run.to_string(),
        _ => ctx.trace().run.to_string(),
    }
}

/// A short human reason from whatever the failing node routed to `fail`.
fn reason_of(payload: &serde_json::Value) -> String {
    // A join node's payload is `{ <source node>: <value> }`; look inside the
    // one arm that fired.
    if let Some(obj) = payload.as_object()
        && obj.len() == 1
        && !["error", "exit_code", "reason"]
            .iter()
            .any(|k| obj.contains_key(*k))
        && let Some(arm) = obj.values().next()
    {
        return reason_of(arm);
    }
    if let Some(e) = payload.get("error").and_then(serde_json::Value::as_str) {
        return e.to_owned();
    }
    if let Some(code) = payload.get("exit_code").and_then(serde_json::Value::as_i64) {
        let out = payload
            .get("output")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let head: String = out.trim().chars().take(200).collect();
        return if head.is_empty() {
            format!("exit code {code}")
        } else {
            format!("exit code {code}: {head}")
        };
    }
    if let Some(r) = payload.get("reason") {
        return format!("provider failure: {r}");
    }
    String::from("step did not complete")
}

impl GraphNode for SopNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let next = |value: serde_json::Value| {
            Ok(Outcome::Next {
                port: PortName::from("ok"),
                value,
            })
        };
        match self.op {
            Op::Begin => {
                for r in &self.requires {
                    let done = input.nodes.get(format!("{r}.end")).is_some();
                    if !done {
                        return Err(
                            self.fail(ctx, &format!("requires step '{r}', which has not finished"))
                        );
                    }
                }
                self.log
                    .event(&self.event(ctx, Level::Info, names::SOP_STEP_STARTED));
                self.prompt.as_ref().map_or_else(
                    || next(input.payload.clone()),
                    |p| next(serde_json::json!({ "prompt": p })),
                )
            }
            Op::Await => {
                let cycle = input
                    .payload
                    .get("cycle")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let gate = self.gate.as_deref().unwrap_or_default();
                let approval = approval_id_for(&run_of(ctx), gate, cycle);
                self.log.event(
                    &self
                        .event(ctx, Level::Info, names::SOP_STEP_AWAITING_APPROVAL)
                        .field("approval", approval),
                );
                next(input.payload.clone())
            }
            Op::End => {
                self.log
                    .event(&self.event(ctx, Level::Info, names::SOP_STEP_FINISHED));
                next(input.payload.clone())
            }
            Op::Fail => {
                let reason = self
                    .cause
                    .clone()
                    .unwrap_or_else(|| reason_of(&input.payload));
                Err(self.fail(ctx, &reason))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use pii_core::{Budget, CapSet, Ctx, TraceCtx};
    use pii_observe::{Recorder, names};

    use super::*;

    fn ctx() -> Ctx {
        Ctx::with_observer(
            CapSet::empty(),
            Budget::unlimited(),
            Arc::new(pii_core::NullObserver),
            TraceCtx {
                run: pii_abi::RunId::from("r1"),
                span: 0,
            },
        )
    }

    fn node(cfg: &serde_json::Value, log: &Arc<Recorder>) -> Box<dyn GraphNode> {
        SopFactory { log: log.clone() }.build(cfg).unwrap()
    }

    fn input(nodes: serde_json::Value) -> NodeInput {
        NodeInput {
            input: serde_json::json!({}),
            payload: serde_json::json!({}),
            nodes,
        }
    }

    fn begin_cfg(requires: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "op": "begin", "sop": "s", "step": "two", "step_kind": "check",
            "requires": requires,
        })
    }

    /// R3: a `requires` whose step never finished fails the run, naming both
    /// steps — and starts nothing.
    #[test]
    fn unmet_requires_fails_loudly_and_never_starts() {
        let log = Arc::new(Recorder::new());
        let err = node(&begin_cfg(&["one"]), &log)
            .run(&ctx(), &input(serde_json::json!({})))
            .unwrap_err()
            .to_string();
        assert!(err.contains("step 'two'") && err.contains("'one'"), "{err}");
        assert_eq!(log.events_named(names::SOP_STEP_FAILED).len(), 1);
        assert_eq!(
            log.events_named(names::SOP_STEP_STARTED),
            [] as [pii_observe::OwnedEvent; 0]
        );
    }

    /// Negative control: a met `requires` continues and starts the step.
    #[test]
    fn met_requires_continues() {
        let log = Arc::new(Recorder::new());
        let out = node(&begin_cfg(&["one"]), &log)
            .run(&ctx(), &input(serde_json::json!({ "one.end": {} })))
            .unwrap();
        assert!(matches!(out, Outcome::Next { ref port, .. } if port.as_str() == "ok"));
        assert_eq!(log.events_named(names::SOP_STEP_STARTED).len(), 1);
        assert_eq!(
            log.events_named(names::SOP_STEP_FAILED),
            [] as [pii_observe::OwnedEvent; 0]
        );
    }

    #[test]
    fn fail_reason_comes_from_the_payload() {
        assert_eq!(
            reason_of(&serde_json::json!({"exit_code": 3, "output": "boom\n"})),
            "exit code 3: boom"
        );
        assert_eq!(reason_of(&serde_json::json!({"error": "x"})), "x");
        // The shape a multi-edge `fail` node really receives.
        assert_eq!(
            reason_of(&serde_json::json!({"a": {"exit_code": 3, "output": "boom"}})),
            "exit code 3: boom"
        );
    }
}
