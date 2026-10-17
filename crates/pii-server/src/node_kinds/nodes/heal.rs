//! `heal_detect` node kind (S0.2): runs an `xtask` gate and records failures.
//!
//! A non-zero exit is reproduced once before a ledger row is written — a
//! single flaky run never creates an entry. **Never granted `FS_WRITE`**
//! (read-only detection).

use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_store::heal::{HealAttemptRun, HealStore};

use pii_graph::node::{GraphNode, NodeFactory};

/// The result of one gate run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    /// Exit was zero.
    pub pass: bool,
    /// Output preview (first 512 bytes) for the ledger detail + signature.
    pub detail: String,
    /// True when the attempt hit the runner's time budget instead of the gate reporting a failure. A timeout is not
    /// evidence of a broken tree, so it must never be recorded as one — measured 2026-10-16: a two-minute cap turned a
    /// busy host into a "reproducible gate failure" with a ledger row and an escalation (doc 064, D2).
    pub timed_out: bool,
}

/// Executes one gate. The production runner shells `cargo xtask <gate>`;
/// tests inject a scripted runner (deterministic, no cargo).
pub trait GateRunner: Send + Sync {
    /// Runs the named gate.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when the ctx lacks PROC; transport errors
    /// otherwise.
    fn run_gate(&self, ctx: &Ctx, gate: &str) -> Result<GateResult, PiiError>;
}

/// The production runner: `/bin/sh -c "cargo xtask <gate>"` in the
/// workspace, via [`pii_core::ProcCap`] — the SAME mechanism the `bash`
/// tool uses (no new process-exec path).
#[derive(Debug, Clone)]
pub struct CargoXtaskRunner {
    /// Workspace the gate runs in.
    pub workspace: std::path::PathBuf,
}

impl GateRunner for CargoXtaskRunner {
    fn run_gate(&self, ctx: &Ctx, gate: &str) -> Result<GateResult, PiiError> {
        let out = ctx.proc()?.run(
            &[
                String::from("/bin/sh"),
                String::from("-c"),
                format!("cargo xtask {gate}"),
            ],
            &self.workspace,
            std::time::Duration::from_mins(2),
        )?;
        let mut detail = String::from_utf8_lossy(&out.stderr).into_owned();
        if detail.trim().is_empty() {
            detail = String::from_utf8_lossy(&out.stdout).into_owned();
        }
        Ok(GateResult {
            pass: out.exit_code == 0 && !out.timed_out,
            detail: truncate(&detail),
            timed_out: out.timed_out,
        })
    }
}

/// The production runner (spec 163): reads the recorded gate verdict, spawns nothing, needs no `PROC`.
///
/// The file is `_tmp/gate/verdicts.jsonl`, one JSON object per line: `{ts_ms, pass, failing_tests}`.
///
/// - newest verdict passed → pass;
/// - newest AND the one before it failed with the same failing set → fail (reproduced across two gate runs);
/// - anything else — no file, no verdict inside [`VERDICT_MAX_AGE_MS`], or a single unconfirmed failure — is reported as
///   `timed_out` (inconclusive): absence of evidence is never recorded as a broken tree.
#[derive(Debug, Clone)]
pub struct VerdictRunner {
    /// Workspace whose `_tmp/gate/verdicts.jsonl` is read.
    pub workspace: std::path::PathBuf,
}

/// A verdict older than this is stale: the watch says so instead of vouching for the tree.
const VERDICT_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;

fn inconclusive(why: &str) -> GateResult {
    GateResult {
        pass: false,
        detail: String::from(why),
        timed_out: true,
    }
}

impl GateRunner for VerdictRunner {
    fn run_gate(&self, ctx: &Ctx, _gate: &str) -> Result<GateResult, PiiError> {
        let path = self.workspace.join("_tmp/gate/verdicts.jsonl");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(inconclusive("stale: no recorded gate verdict"));
        };
        let mut recent = text
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .rev();
        let Some(newest) = recent.next() else {
            return Ok(inconclusive("stale: no recorded gate verdict"));
        };
        let ts = newest
            .get("ts_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if ctx.now_ms().saturating_sub(ts) > VERDICT_MAX_AGE_MS {
            return Ok(inconclusive(
                "stale: newest gate verdict is older than 24 h",
            ));
        }
        let passed = |v: &serde_json::Value| {
            v.get("pass").and_then(serde_json::Value::as_bool) == Some(true)
        };
        if passed(&newest) {
            return Ok(GateResult {
                pass: true,
                detail: String::new(),
                timed_out: false,
            });
        }
        let failing = |v: &serde_json::Value| v.get("failing_tests").cloned().unwrap_or_default();
        match recent.next() {
            Some(prev) if !passed(&prev) && failing(&prev) == failing(&newest) => Ok(GateResult {
                pass: false,
                detail: truncate(&format!(
                    "gate failed in two consecutive runs: {}",
                    failing(&newest)
                )),
                timed_out: false,
            }),
            _ => Ok(inconclusive(
                "unconfirmed: one failed gate verdict, not yet reproduced",
            )),
        }
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(512).collect()
}

/// Registered into [`crate::engine::registry`]; every `heal_detect` node shares the
/// heal ledger and one runner.
pub struct HealFactory {
    /// The shared heal-attempt ledger.
    pub heals: Arc<HealStore>,
    /// The gate runner (production: cargo xtask).
    pub runner: Arc<dyn GateRunner>,
}

impl NodeFactory for HealFactory {
    fn kind(&self) -> &'static str {
        "heal_detect"
    }

    fn effects(&self) -> pii_abi::Effects {
        // Reads external state and may write the ledger — serialised.
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        // Reads a recorded verdict and spawns nothing: no PROC, and never FS_WRITE (spec 163).
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![
            PortName::from("ok"),
            PortName::from("detected"),
            PortName::from("flake"),
            // A timeout is not a failure: it is inconclusive, and it finishes the run with no ledger row (doc 064, D2).
            PortName::from("inconclusive"),
        ])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let gate = cfg
            .get("gate")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("heal_detect requires 'gate' (string)")))?
            .to_owned();
        Ok(Box::new(HealDetectNode {
            gate,
            heals: Arc::clone(&self.heals),
            runner: Arc::clone(&self.runner),
        }))
    }
}

/// The node instance (S0.2): ok on pass; reproduce-then-route on failure.
struct HealDetectNode {
    gate: String,
    heals: Arc<HealStore>,
    runner: Arc<dyn GateRunner>,
}

impl GraphNode for HealDetectNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        // Attempt 1.
        let first = self.runner.run_gate(ctx, &self.gate)?;
        if first.pass {
            return Ok(Outcome::Next {
                port: PortName::from("ok"),
                value: input.payload.clone(),
            });
        }
        // Reproduce before recording (guardrail): run the SAME check again.
        let second = self.runner.run_gate(ctx, &self.gate)?;
        // A timeout is inconclusive, not a failure: the tree may be fine and the host merely busy. It writes NO ledger
        // row and never escalates a human gate. (Doc 064, D2.)
        if first.timed_out || second.timed_out {
            let detail = if first.timed_out {
                first.detail
            } else {
                second.detail
            };
            return Ok(Outcome::Next {
                port: PortName::from("inconclusive"),
                value: serde_json::json!({"reason": "timeout", "detail": detail}),
            });
        }
        if second.pass {
            // Flake — no ledger row.
            return Ok(Outcome::Next {
                port: PortName::from("flake"),
                value: input.payload.clone(),
            });
        }
        // Both failed: reproducible. Write ONE ledger row for the signature.
        let run_id = ctx.trace().run.to_string();
        let detail = if second.detail.is_empty() {
            first.detail
        } else {
            second.detail
        };
        let sig = signature(&self.gate, &detail);
        self.heals.record_run(
            &sig,
            &self.gate,
            HealAttemptRun {
                run_id,
                outcome: pii_store::heal::HealState::Detected,
                ts_ms: ctx.now_ms(),
                detail: detail.clone(),
            },
        )?;
        Ok(Outcome::Next {
            port: PortName::from("detected"),
            value: serde_json::json!({"signature_hash": sig, "detail": detail}),
        })
    }
}

/// `blake3(gate, first-N bytes of failure output)` — deterministic so
/// re-detecting the SAME failure updates the same row (S0.1).
#[must_use]
pub fn signature(gate: &str, detail: &str) -> String {
    let bytes = format!("{gate}:{detail}");
    pii_store::hash(bytes.as_bytes())
}

/// `heal_record` node kind (spec 33 R4): writes the repair evidence
/// (`gate_before`/`gate_after`/`patch_hash`/`model`/`tokens_in`/
/// `tokens_out`) to the heal ledger.
///
/// Placed on the SUCCESS path only (`route_retest.ok`) — a still-broken
/// retest never reaches a node here (the run fails structurally at
/// `loop_budget`, see [`record_loop_budget_escalation`]), so this node
/// only ever records `HealState::Fixed`.
///
/// Reads `$.input.signature` (required — the signature the run was spawned
/// for) and `$.input.gate` (optional, defaults to `"heal-fix"`), plus the
/// `fix_node`'s `model`/`tokens_in`/`tokens_out` and the `diff_node`'s
/// `stdout` (the patch text) from the blackboard.
pub struct HealRecordFactory {
    /// The shared heal-attempt ledger.
    pub heals: Arc<HealStore>,
}

impl NodeFactory for HealRecordFactory {
    fn kind(&self) -> &'static str {
        "heal_record"
    }

    fn effects(&self) -> pii_abi::Effects {
        pii_abi::Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::empty()
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let gate_before = cfg
            .get("gate_before")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("heal_record requires 'gate_before'")))?
            .to_owned();
        let gate_after = cfg
            .get("gate_after")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("heal_record requires 'gate_after'")))?
            .to_owned();
        let fix_node = cfg
            .get("fix_node")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("heal_record requires 'fix_node'")))?
            .to_owned();
        let diff_node = cfg
            .get("diff_node")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("heal_record requires 'diff_node'")))?
            .to_owned();
        Ok(Box::new(HealRecordNode {
            heals: Arc::clone(&self.heals),
            gate_before,
            gate_after,
            fix_node,
            diff_node,
        }))
    }
}

struct HealRecordNode {
    heals: Arc<HealStore>,
    gate_before: String,
    gate_after: String,
    fix_node: String,
    diff_node: String,
}

impl GraphNode for HealRecordNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let sig = input
            .input
            .get("signature")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("heal_record requires input.signature")))?
            .to_owned();
        let gate = input
            .input
            .get("gate")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("heal-fix")
            .to_owned();
        let fix = input.nodes.get(&self.fix_node);
        let model = fix
            .and_then(|f| f.get("model"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let tokens_in = fix
            .and_then(|f| f.get("tokens_in"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let tokens_out = fix
            .and_then(|f| f.get("tokens_out"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let diff = input
            .nodes
            .get(&self.diff_node)
            .and_then(|d| d.get("stdout"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let patch_hash = pii_store::hash(diff.as_bytes());
        let run_id = ctx.trace().run.to_string();
        let attempt = self.heals.record_repair(
            &sig,
            &gate,
            HealAttemptRun {
                run_id,
                outcome: pii_store::heal::HealState::Fixed,
                ts_ms: ctx.now_ms(),
                detail: truncate(diff),
            },
            Some(self.gate_before.clone()),
            Some(self.gate_after.clone()),
            Some(patch_hash),
            model,
            tokens_in,
            tokens_out,
        )?;
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({ "signature_hash": attempt.signature_hash }),
        })
    }
}

/// Records the loop-budget-exceeded escalation (spec 33 R3/R4/G5/G6).
///
/// A still-broken retest after `loop_budget` fails the run STRUCTURALLY,
/// at the engine's dispatch check (`pii_graph::run::run`'s batch-readiness
/// loop) — no node in the graph ever runs after that point, so no
/// `heal_record` node can fire. This is the one ledger write that must
/// happen outside the graph, from the run's terminal instead.
///
/// Only fires for `starter/heal-fix` runs whose failure names
/// `loop_budget exceeded` and whose input carries a `signature` — anything
/// else is left untouched (never a silent write for an unrelated graph).
pub fn record_loop_budget_escalation(
    heals: &HealStore,
    graph_name: &str,
    input: &serde_json::Value,
    terminal: Option<&pii_abi::Terminal>,
    run_id: &str,
    now_ms: u64,
) {
    if graph_name != "starter/heal-fix" {
        return;
    }
    let Some(pii_abi::Terminal::Failure { error }) = terminal else {
        return;
    };
    if !error.contains("loop_budget exceeded") {
        return;
    }
    let Some(sig) = input.get("signature").and_then(serde_json::Value::as_str) else {
        return;
    };
    let gate = input
        .get("gate")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("heal-fix");
    let _ = heals.record_repair(
        sig,
        gate,
        HealAttemptRun {
            run_id: run_id.to_owned(),
            outcome: pii_store::heal::HealState::Escalated,
            ts_ms: now_ms,
            detail: truncate(error),
        },
        None,
        None,
        None,
        None,
        0,
        0,
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use parking_lot::Mutex;

    use pii_abi::{Outcome, PiiError};
    use pii_core::{Budget, CapKind, CapSet, Ctx, TraceCtx};
    use pii_store::heal::HealStore;

    use pii_graph::node::NodeFactory as _;

    use super::{GateResult, GateRunner, HealFactory};

    /// A scripted runner: `results` are consumed in order, then Ok.
    struct Scripted {
        results: Mutex<Vec<GateResult>>,
    }

    impl GateRunner for Scripted {
        fn run_gate(&self, _ctx: &Ctx, _gate: &str) -> Result<GateResult, PiiError> {
            let mut r = self.results.lock();
            Ok(if r.is_empty() {
                GateResult {
                    pass: true,
                    detail: String::new(),
                    timed_out: false,
                }
            } else {
                r.remove(0)
            })
        }
    }

    fn ctx() -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::PROC;
        Ctx::with_observer(
            caps,
            Budget::unlimited(),
            std::sync::Arc::new(pii_core::NullObserver),
            TraceCtx {
                run: pii_abi::RunId::from("r-test"),
                span: 0,
            },
        )
    }

    fn factory(results: Vec<GateResult>) -> HealFactory {
        let dir =
            pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "heal-node-").unwrap();
        let heals = std::sync::Arc::new(HealStore::open(&dir.path().join("heal.redb")).unwrap());
        HealFactory {
            heals,
            runner: std::sync::Arc::new(Scripted {
                results: Mutex::new(results),
            }),
        }
    }

    fn fail(msg: &str) -> GateResult {
        GateResult {
            pass: false,
            detail: String::from(msg),
            timed_out: false,
        }
    }

    #[test]
    fn passing_gate_routes_ok_and_writes_nothing() {
        let f = factory(vec![GateResult {
            pass: true,
            detail: String::new(),
            timed_out: false,
        }]);
        let node = f
            .build(&serde_json::json!({"gate": "reachability-check"}))
            .unwrap();
        let input = pii_abi::NodeInput {
            input: serde_json::Value::Null,
            payload: serde_json::Value::Null,
            nodes: serde_json::json!({}),
        };
        let out = node.run(&ctx(), &input).unwrap();
        assert!(matches!(out, Outcome::Next { port, .. } if port.as_str() == "ok"));
        assert!(f.heals.list_attempts().unwrap().is_empty());
    }

    #[test]
    fn failing_gate_both_times_routes_detected_and_writes_one_row() {
        let f = factory(vec![fail("gate failed once"), fail("gate failed again")]);
        let node = f
            .build(&serde_json::json!({"gate": "quality-gate"}))
            .unwrap();
        let input = pii_abi::NodeInput {
            input: serde_json::Value::Null,
            payload: serde_json::Value::Null,
            nodes: serde_json::json!({}),
        };
        let out = node.run(&ctx(), &input).unwrap();
        match out {
            Outcome::Next { port, value } => {
                assert_eq!(port.as_str(), "detected");
                let sig = value.get("signature_hash").unwrap().as_str().unwrap();
                assert_eq!(f.heals.list_attempts().unwrap().len(), 1);
                assert_eq!(f.heals.get_attempt(sig).unwrap().unwrap().attempts.len(), 1);
            }
            other => panic!("want detected, got {other:?}"),
        }
    }

    #[test]
    fn flake_routes_flake_and_writes_nothing() {
        let f = factory(vec![
            fail("transient failure"),
            GateResult {
                pass: true,
                detail: String::new(),
                timed_out: false,
            },
        ]);
        let node = f.build(&serde_json::json!({"gate": "lint-files"})).unwrap();
        let input = pii_abi::NodeInput {
            input: serde_json::Value::Null,
            payload: serde_json::Value::Null,
            nodes: serde_json::json!({}),
        };
        let out = node.run(&ctx(), &input).unwrap();
        assert!(matches!(out, Outcome::Next { port, .. } if port.as_str() == "flake"));
        assert!(f.heals.list_attempts().unwrap().is_empty());
    }
}
