//! The queue's graph nodes (spec 208).
//!
//! `select` gates the next work item, `judge` runs its criteria, and `record` writes the verdict back to the store.
//! The dispatch in between is the `coder` node (spec 207), which reads its spec path from the selected item.
//! Every node reads the blackboard (`input.nodes`), keyed by node id, so `judge` and `record` can reach the item
//! that `select` claimed without the graph copying it along each edge.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::node::{GraphNode, NodeFactory};
use pii_store::MemStore;
use pii_store::queue::{Claim, ItemStatus, QueueStore};

use super::agent_worker::{Verdict, learn_fact};
use super::judge_spec::SpecJudgeNode;

/// How long one criterion command may run before it is killed.
const CRITERION_TIMEOUT: Duration = Duration::from_mins(10);

/// The blackboard entry of the `select` node.
fn selected(input: &NodeInput) -> Option<&serde_json::Value> {
    input.nodes.get("select").and_then(|v| v.get("item"))
}

fn port(name: &str, value: serde_json::Value) -> Outcome {
    Outcome::Next {
        port: PortName::from(name),
        value,
    }
}

/// Factory for `select`: claims the first pending item whose needs are done, or reports why it cannot.
pub struct SelectFactory {
    /// The queue store the claim is made against.
    pub store: Arc<QueueStore>,
}

struct SelectNode {
    store: Arc<QueueStore>,
}

impl GraphNode for SelectNode {
    fn run(&self, _ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        Ok(match self.store.claim_next("select")? {
            Claim::Claimed(item) => port(
                "ok",
                serde_json::json!({
                    "outcome": "claimed",
                    "id": item.id,
                    "item": item,
                }),
            ),
            Claim::Blocked(blockers) => {
                // Blocked names every unmet edge; it is never reported as an empty queue.
                let summary: Vec<String> = blockers
                    .iter()
                    .map(|b| format!("{} waits on {} ({})", b.item, b.needs, b.need_status))
                    .collect();
                port(
                    "blocked",
                    serde_json::json!({
                        "outcome": "blocked",
                        "waiting": blockers,
                        "summary": summary.join("; "),
                    }),
                )
            }
            Claim::Empty => port("empty", serde_json::json!({ "outcome": "queue-empty" })),
            Claim::AlreadyRunning(id) => port(
                "running",
                serde_json::json!({ "outcome": "already-running", "id": id }),
            ),
        })
    }
}

impl NodeFactory for SelectFactory {
    fn kind(&self) -> &'static str {
        "select"
    }

    fn is_effect(&self) -> bool {
        true
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(["ok", "blocked", "empty", "running"]
            .into_iter()
            .map(PortName::from)
            .collect())
    }

    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(SelectNode {
            store: Arc::clone(&self.store),
        }))
    }
}

/// Factory for `judge`: runs the selected item's criteria in `workdir`, in order. The first non-zero stops the
/// node and routes to `fail`, naming the criterion.
pub struct JudgeFactory;

struct JudgeNode {
    workdir: PathBuf,
}

impl GraphNode for JudgeNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let Some(item) = selected(input) else {
            return Ok(Outcome::Fail {
                error: "judge: no item was selected".to_owned(),
            });
        };
        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let criteria: Vec<String> = item
            .get("criteria")
            .and_then(serde_json::Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        for command in &criteria {
            let shell = ["/bin/sh".to_owned(), "-c".to_owned(), command.clone()];
            let rc = ctx
                .proc()?
                .run(&shell, &self.workdir, CRITERION_TIMEOUT)?
                .exit_code;
            if rc != 0 {
                return Ok(port(
                    "fail",
                    serde_json::json!({
                        "verdict": "failed",
                        "id": id,
                        "criterion": command,
                        "rc": rc,
                        "reason": format!("criterion failed: {command} (rc {rc})"),
                    }),
                ));
            }
        }
        Ok(port(
            "ok",
            serde_json::json!({ "verdict": "passed", "id": id, "criteria": criteria.len() }),
        ))
    }
}

impl NodeFactory for JudgeFactory {
    fn kind(&self) -> &'static str {
        "judge"
    }

    fn is_effect(&self) -> bool {
        true
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::PROC
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("fail")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        // Spec mode (spec 259) resolves its keys at run time, so it needs no literal `workdir` here.
        if cfg.get("spec").is_some() {
            return Ok(Box::new(SpecJudgeNode { cfg: cfg.clone() }));
        }
        let workdir = cfg
            .get("workdir")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| PiiError::Internal("judge: config needs `workdir`".to_owned()))?;
        Ok(Box::new(JudgeNode { workdir }))
    }
}

/// Factory for `record`: writes the item's verdict to the queue store.
///
/// A pass marks the item `done`; a failure or a refused dispatch parks it with the reason, journalled by the store.
/// When `memory` is set, the outcome is also learned as one memory document naming the item and the failing
/// criterion (spec 208 R6).
pub struct RecordFactory {
    /// The queue store the status is written to.
    pub store: Arc<QueueStore>,
    /// The memory the outcome is learned into; `None` in lint, where nothing is written.
    pub memory: Option<Arc<MemStore>>,
}

struct RecordNode {
    store: Arc<QueueStore>,
    memory: Option<Arc<MemStore>>,
}

/// The string field `key` of `value`, or an empty string.
fn field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl GraphNode for RecordNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let Some(item) = selected(input) else {
            return Ok(Outcome::Fail {
                error: "record: no item was selected".to_owned(),
            });
        };
        let id = field(item, "id");
        // The verdict is the judge's, or the dispatch's refusal when the judge never ran.
        let (status, verdict) = match (input.nodes.get("judge"), input.nodes.get("dispatch")) {
            (Some(judge), _) if judge["verdict"] == "passed" => (
                ItemStatus::Done,
                Verdict::Passed {
                    report: String::new(),
                    criteria: Vec::new(),
                },
            ),
            (Some(judge), _) => {
                let failing = judge.get("criterion").and_then(serde_json::Value::as_str);
                (
                    ItemStatus::Parked,
                    Verdict::Failed {
                        reason: field(judge, "reason"),
                        failing: failing.map(str::to_owned),
                    },
                )
            }
            (None, Some(refused)) => (
                ItemStatus::Parked,
                Verdict::Failed {
                    reason: field(refused, "reason"),
                    failing: None,
                },
            ),
            (None, None) => (
                ItemStatus::Parked,
                Verdict::Failed {
                    reason: "no verdict was reached".to_owned(),
                    failing: None,
                },
            ),
        };
        let reason = match &verdict {
            Verdict::Failed { reason, .. } => Some(reason.as_str()),
            Verdict::Passed { .. } => None,
        };
        self.store.set_status(&id, status, reason, "record")?;
        if let Some(memory) = &self.memory {
            let spec = format!("{id} {}", field(item, "spec"));
            let now_ms = crate::engine::runs::learn_now_secs() * 1000;
            memory.put(&learn_fact(&spec, "coder", "judge", &verdict, now_ms))?;
        }
        Ok(port(
            "ok",
            serde_json::json!({ "id": id, "status": status.name(), "reason": reason }),
        ))
    }
}

impl NodeFactory for RecordFactory {
    fn kind(&self) -> &'static str {
        "record"
    }

    fn is_effect(&self) -> bool {
        true
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, _cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(RecordNode {
            store: Arc::clone(&self.store),
            memory: self.memory.clone(),
        }))
    }
}
