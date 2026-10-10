//! `hal` node kind (spec 121): drives a device through the daemon's
//! [`pii_hal::Hal`].
//!
//! Config: `op` plus its arguments; every argument may be a `$.`-expression.
//!
//! | `op`                         | arguments                       | `ok` value                   |
//! |------------------------------|---------------------------------|------------------------------|
//! | `claim`                      | `pin`, `dir` (`in`/`out`)       | `{op, pin, dir}`             |
//! | `release`                    | `pin`                           | `{op, pin}`                  |
//! | `read`                       | `pin`                           | `{op, pin, level, high}`     |
//! | `write`                      | `pin`, `level` (`high`/`low`)   | `{op, pin, level}`           |
//! | `claim_i2c` / `release_i2c`  | `bus`, `addr`                   | `{op, bus, addr}`            |
//! | `i2c`                        | `bus`, `addr`, `write`?, `read_len`? | `{op, bus, addr, read}` |
//! | `claim_spi` / `release_spi`  | `bus`                           | `{op, bus}`                  |
//! | `spi`                        | `bus`, `tx`                     | `{op, bus, rx}`              |
//!
//! There is no `err` port: a refusal (unclaimed, out of scope, double claim, no
//! `[hal]` backend configured) fails the run, loudly, with the HAL's `Denied`.

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};
use pii_hal::{Direction, Hal, HalHub, PinLevel};
use pii_observe::{Event, Level, Observer, names};

const OPS: &[&str] = &[
    "claim",
    "release",
    "read",
    "write",
    "claim_i2c",
    "release_i2c",
    "i2c",
    "claim_spi",
    "release_spi",
    "spi",
];

/// Registered by [`crate::Engine::new`]; every `hal` node shares the hub the
/// daemon fills from `[hal]`.
pub struct HalFactory {
    /// The daemon's HAL slot (empty unless `[hal]` is configured).
    pub hub: Arc<HalHub>,
    /// Receives the `hal.error` event when no backend is configured.
    pub log: Arc<dyn Observer>,
}

struct HalNode {
    hub: Arc<HalHub>,
    log: Arc<dyn Observer>,
    op: String,
    cfg: serde_json::Value,
}

impl NodeFactory for HalFactory {
    fn kind(&self) -> &'static str {
        "hal"
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::HW
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let op = cfg
            .get("op")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("hal requires 'op' (string)")))?;
        if !OPS.contains(&op) {
            return Err(PiiError::Invalid(format!(
                "hal: unknown op '{op}' (known: {})",
                OPS.join(", ")
            )));
        }
        Ok(Box::new(HalNode {
            hub: Arc::clone(&self.hub),
            log: Arc::clone(&self.log),
            op: op.to_owned(),
            cfg: cfg.clone(),
        }))
    }
}

/// One argument of the node config, with `$.` expressions resolved.
struct Args<'a> {
    cfg: &'a serde_json::Value,
    ec: serde_json::Value,
}

impl Args<'_> {
    fn get(&self, key: &str) -> Result<Option<serde_json::Value>, PiiError> {
        self.cfg
            .get(key)
            .map(|v| eval_or_literal(&self.ec, v))
            .transpose()
    }

    fn need(&self, key: &str) -> Result<serde_json::Value, PiiError> {
        self.get(key)?
            .ok_or_else(|| PiiError::Invalid(format!("hal: this op requires '{key}'")))
    }

    fn int<T: TryFrom<u64>>(&self, key: &str) -> Result<T, PiiError> {
        self.need(key)?
            .as_u64()
            .and_then(|n| T::try_from(n).ok())
            .ok_or_else(|| {
                PiiError::Invalid(format!("hal: '{key}' must be a small unsigned integer"))
            })
    }

    fn bytes(&self, key: &str) -> Result<Vec<u8>, PiiError> {
        let Some(v) = self.get(key)? else {
            return Ok(Vec::new());
        };
        let bad = || PiiError::Invalid(format!("hal: '{key}' must be an array of bytes (0..=255)"));
        v.as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|b| {
                b.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(bad)
            })
            .collect()
    }

    fn level(&self) -> Result<PinLevel, PiiError> {
        let v = self.need("level")?;
        let parsed = match &v {
            serde_json::Value::Bool(b) => Some(PinLevel::from(*b)),
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| match n {
                0 => Some(PinLevel::Low),
                1 => Some(PinLevel::High),
                _ => None,
            }),
            serde_json::Value::String(s) => PinLevel::parse(s),
            _ => None,
        };
        parsed
            .ok_or_else(|| PiiError::Invalid(format!("hal: 'level' must be high or low, got {v}")))
    }
}

impl HalNode {
    fn apply(&self, hal: &Hal, a: &Args<'_>) -> Result<serde_json::Value, PiiError> {
        let op = self.op.as_str();
        let out = match op {
            "claim" => {
                let pin: u32 = a.int("pin")?;
                let dir = a.need("dir")?;
                let dir = dir.as_str().and_then(Direction::parse).ok_or_else(|| {
                    PiiError::Invalid(format!("hal: 'dir' must be in or out, got {dir}"))
                })?;
                hal.claim_pin(pin, dir)?;
                serde_json::json!({ "pin": pin, "dir": dir.as_str() })
            }
            "release" => {
                let pin: u32 = a.int("pin")?;
                hal.release_pin(pin)?;
                serde_json::json!({ "pin": pin })
            }
            "read" => {
                let pin: u32 = a.int("pin")?;
                let level = hal.read_pin(pin)?;
                serde_json::json!({ "pin": pin, "level": level.as_str(), "high": level.is_high() })
            }
            "write" => {
                let pin: u32 = a.int("pin")?;
                let level = a.level()?;
                hal.write_pin(pin, level)?;
                serde_json::json!({ "pin": pin, "level": level.as_str() })
            }
            "claim_i2c" | "release_i2c" => {
                let (bus, addr): (u8, u8) = (a.int("bus")?, a.int("addr")?);
                if op == "claim_i2c" {
                    hal.claim_i2c(bus, addr)?;
                } else {
                    hal.release_i2c(bus, addr)?;
                }
                serde_json::json!({ "bus": bus, "addr": addr })
            }
            "i2c" => {
                let (bus, addr): (u8, u8) = (a.int("bus")?, a.int("addr")?);
                let read_len: usize = if a.get("read_len")?.is_some() {
                    a.int("read_len")?
                } else {
                    0
                };
                let read = hal.i2c_transaction(bus, addr, &a.bytes("write")?, read_len)?;
                serde_json::json!({ "bus": bus, "addr": addr, "read": read })
            }
            "claim_spi" | "release_spi" => {
                let bus: u8 = a.int("bus")?;
                if op == "claim_spi" {
                    hal.claim_spi(bus)?;
                } else {
                    hal.release_spi(bus)?;
                }
                serde_json::json!({ "bus": bus })
            }
            _ => {
                let bus: u8 = a.int("bus")?;
                let rx = hal.spi_transfer(bus, &a.bytes("tx")?)?;
                serde_json::json!({ "bus": bus, "rx": rx })
            }
        };
        Ok(with_op(out, op))
    }
}

fn with_op(mut v: serde_json::Value, op: &str) -> serde_json::Value {
    if let Some(m) = v.as_object_mut() {
        m.insert(String::from("op"), serde_json::Value::from(op));
    }
    v
}

impl GraphNode for HalNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let hal = self.hub.require().inspect_err(|e| {
            self.log.event(
                &Event::new("hal", Level::Error, names::HAL_ERROR)
                    .field("op", self.op.as_str())
                    .field("reason", "no_backend")
                    .field("error", e.to_string()),
            );
        })?;
        let args = Args {
            cfg: &self.cfg,
            ec: serde_json::json!({
                "input": input.input, "payload": input.payload, "nodes": input.nodes,
            }),
        };
        Ok(Outcome::Next {
            port: PortName::from("ok"),
            value: self.apply(&hal, &args)?,
        })
    }
}
