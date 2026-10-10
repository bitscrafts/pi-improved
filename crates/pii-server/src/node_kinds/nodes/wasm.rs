//! `wasm` node kind (spec 122): runs a plugin module through the daemon's
//! [`pii_wasm::WasmHost`].
//!
//! Config: `module` (a path inside the `[wasm]` dir), optional `entry`
//! (default `run`), `input` (default: the node's input payload), `timeout_ms`
//! (may only lower the host's budget) and `fuel` (`true` = the host's default
//! ceiling, or a number of units; absent = no fuel metering). `module`,
//! `input`, `entry`, `timeout_ms` and `fuel` may be `$.`-expressions.
//!
//! The `ok` value is the JSON the module returned. There is no `err` port: a
//! refusal, trap, timeout, fuel or memory exhaustion fails the run, loudly,
//! with the host's named error.

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};
use pii_observe::{Event, Level, Observer, names};
use pii_wasm::{CallOpts, Fuel, WasmHub};

/// Registered by [`crate::Engine::new`]; every `wasm` node shares the hub the
/// daemon fills from `[wasm]`.
pub struct WasmFactory {
    /// The daemon's plugin-host slot (empty unless `[wasm]` is configured).
    pub hub: Arc<WasmHub>,
    /// Receives the `wasm.refusal` event when no host is configured.
    pub log: Arc<dyn Observer>,
}

struct WasmNode {
    hub: Arc<WasmHub>,
    log: Arc<dyn Observer>,
    cfg: serde_json::Value,
}

impl NodeFactory for WasmFactory {
    fn kind(&self) -> &'static str {
        "wasm"
    }

    // A plugin has no ambient authority: it can compute, and nothing else.
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
        if !cfg.get("module").is_some_and(serde_json::Value::is_string) {
            return Err(PiiError::Invalid(String::from(
                "wasm requires 'module' (string)",
            )));
        }
        Ok(Box::new(WasmNode {
            hub: Arc::clone(&self.hub),
            log: Arc::clone(&self.log),
            cfg: cfg.clone(),
        }))
    }
}

impl WasmNode {
    fn arg(
        &self,
        ec: &serde_json::Value,
        key: &str,
    ) -> Result<Option<serde_json::Value>, PiiError> {
        self.cfg
            .get(key)
            .map(|v| eval_or_literal(ec, v))
            .transpose()
    }

    fn opts(&self, ec: &serde_json::Value) -> Result<CallOpts, PiiError> {
        let bad =
            |key: &str, want: &str| PiiError::Invalid(format!("wasm: '{key}' must be {want}"));
        let entry = match self.arg(ec, "entry")? {
            None => None,
            Some(v) => Some(
                v.as_str()
                    .ok_or_else(|| bad("entry", "a string"))?
                    .to_owned(),
            ),
        };
        let timeout = match self.arg(ec, "timeout_ms")? {
            None => None,
            Some(v) => {
                Some(Duration::from_millis(v.as_u64().ok_or_else(|| {
                    bad("timeout_ms", "a whole number of milliseconds")
                })?))
            }
        };
        let fuel = match self.arg(ec, "fuel")? {
            None | Some(serde_json::Value::Bool(false)) => Fuel::Off,
            Some(serde_json::Value::Bool(true)) => Fuel::Default,
            Some(v) => Fuel::Limit(
                v.as_u64()
                    .ok_or_else(|| bad("fuel", "true, false or a number of units"))?,
            ),
        };
        Ok(CallOpts {
            entry,
            timeout,
            fuel,
        })
    }
}

impl GraphNode for WasmNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let host = self.hub.require().inspect_err(|e| {
            self.log.event(
                &Event::new("wasm", Level::Error, names::WASM_REFUSAL)
                    .field(
                        "module",
                        self.cfg
                            .get("module")
                            .and_then(|m| m.as_str())
                            .unwrap_or(""),
                    )
                    .field(
                        "entry",
                        self.cfg
                            .get("entry")
                            .and_then(|m| m.as_str())
                            .unwrap_or("run"),
                    )
                    .field("reason", "not_configured")
                    .field("error", e.to_string()),
            );
        })?;
        let ec = serde_json::json!({
            "input": input.input, "payload": input.payload, "nodes": input.nodes,
        });
        let module = self
            .arg(&ec, "module")?
            .and_then(|m| m.as_str().map(str::to_owned))
            .ok_or_else(|| PiiError::Invalid(String::from("wasm: 'module' must be a string")))?;
        let payload = self
            .arg(&ec, "input")?
            .unwrap_or_else(|| input.payload.clone());
        let value = host.call(&module, &payload, &self.opts(&ec)?)?;
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value,
        })
    }
}
