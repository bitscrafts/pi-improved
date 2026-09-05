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
        })
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(512).collect()
}

/// Registered into [`crate::registry`]; every `heal_detect` node shares the
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
        // PROC only — never FS_WRITE (guardrail: read-only detection).
        CapKind::PROC
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![
            PortName::from("ok"),
            PortName::from("detected"),
            PortName::from("flake"),
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
                outcome: String::from("detected"),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use parking_lot::Mutex;

    use pii_abi::{Outcome, PiiError};
    use pii_core::{Budget, CapKind, CapSet, Ctx, TraceCtx};
    use pii_store::heal::HealStore;
    use tempfile::Builder;

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
        let dir = Builder::new()
            .prefix("heal-node-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
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
        }
    }

    #[test]
    fn passing_gate_routes_ok_and_writes_nothing() {
        let f = factory(vec![GateResult {
            pass: true,
            detail: String::new(),
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
