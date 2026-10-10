//! `mqtt` node kind (spec 117): publishes to a configured broker connection.
//!
//! Config: `{ broker, op = "publish", topic, qos, retain, payload? }`. `qos`
//! (0..=2) and `retain` are **required** — never implicit. `topic` and
//! `payload` may be `$.`-expressions. The default payload is the
//! predecessor's output (`$.payload`: the value itself with one inbound edge,
//! an object keyed by node id with several). A string is published as its
//! bytes; any other JSON value as its JSON text. Ports: `ok` `{topic, bytes}`,
//! `err` `{error}`. The subscribe direction is the `[[mqtt]]` section: it
//! starts a graph per message.

use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_channels::mqtt::MqttHub;
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};

struct MqttNode {
    hub: Arc<MqttHub>,
    broker: String,
    topic: serde_json::Value,
    qos: u8,
    retain: bool,
    payload: Option<serde_json::Value>,
}

impl MqttNode {
    fn publish(&self, input: &NodeInput) -> Result<serde_json::Value, PiiError> {
        let ec = serde_json::json!({
            "input": input.input, "payload": input.payload, "nodes": input.nodes,
        });
        let topic = match eval_or_literal(&ec, &self.topic)? {
            serde_json::Value::String(t) if !t.is_empty() => t,
            other => {
                return Err(PiiError::Invalid(format!(
                    "mqtt: topic must resolve to a non-empty string, got {other}"
                )));
            }
        };
        let value = match &self.payload {
            Some(p) => eval_or_literal(&ec, p)?,
            None => input.payload.clone(),
        };
        let bytes = match value {
            serde_json::Value::String(s) => s.into_bytes(),
            other => other.to_string().into_bytes(),
        };
        let conn = self.hub.get(&self.broker).ok_or_else(|| {
            PiiError::NotFound(format!("mqtt broker '{}' is not configured", self.broker))
        })?;
        conn.publish(&topic, &bytes, self.qos, self.retain)?;
        Ok(serde_json::json!({ "topic": topic, "bytes": bytes.len() }))
    }
}

impl GraphNode for MqttNode {
    fn run(&self, _ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let (port, value) = match self.publish(input) {
            Ok(v) => ("ok", v),
            Err(e) => ("err", serde_json::json!({ "error": e.to_string() })),
        };
        Ok(Outcome::Next {
            port: PortName::from(port),
            value,
        })
    }
}

/// Registered by [`crate::Engine::new`]; every `mqtt` node shares the hub the
/// daemon fills from `[[mqtt]]`.
pub struct MqttFactory {
    /// The daemon's MQTT connections.
    pub hub: Arc<MqttHub>,
}

impl NodeFactory for MqttFactory {
    fn kind(&self) -> &'static str {
        "mqtt"
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::NET
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let need = |k: &str| {
            cfg.get(k)
                .ok_or_else(|| PiiError::Invalid(format!("mqtt node requires '{k}'")))
        };
        let broker = need("broker")?
            .as_str()
            .ok_or_else(|| PiiError::Invalid("mqtt: 'broker' must be a string".into()))?
            .to_owned();
        match need("op")?.as_str() {
            Some("publish") => {}
            _ => {
                return Err(PiiError::Invalid(
                    "mqtt: only op = \"publish\" exists; subscribing is the [[mqtt]] config section"
                        .into(),
                ));
            }
        }
        let qos = need("qos")?
            .as_u64()
            .and_then(|q| u8::try_from(q).ok())
            .filter(|q| *q <= 2)
            .ok_or_else(|| PiiError::Invalid("mqtt: 'qos' must be 0, 1 or 2".into()))?;
        let retain = need("retain")?
            .as_bool()
            .ok_or_else(|| PiiError::Invalid("mqtt: 'retain' must be true or false".into()))?;
        Ok(Box::new(MqttNode {
            hub: Arc::clone(&self.hub),
            broker,
            topic: need("topic")?.clone(),
            qos,
            retain,
            payload: cfg.get("payload").cloned(),
        }))
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

    use super::*;
    use serde_json::json;

    fn build(cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        MqttFactory {
            hub: Arc::new(MqttHub::default()),
        }
        .build(cfg)
    }

    fn base() -> serde_json::Value {
        json!({"broker": "b", "op": "publish", "topic": "t", "qos": 1, "retain": false})
    }

    #[test]
    fn qos_and_retain_are_required_never_implicit() {
        assert!(build(&base()).is_ok());
        for key in ["qos", "retain", "topic", "broker", "op"] {
            let mut c = base();
            c.as_object_mut().unwrap().remove(key);
            assert!(build(&c).is_err(), "missing {key} must be refused");
        }
        let mut c = base();
        c["qos"] = json!(3);
        assert!(build(&c).is_err());
        c = base();
        c["op"] = json!("subscribe");
        assert!(build(&c).is_err());
    }

    #[test]
    fn an_unconfigured_broker_goes_out_the_err_port_not_a_panic() {
        let node = build(&base()).unwrap();
        let ctx = Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
        let input = NodeInput {
            input: json!({}),
            payload: json!("x"),
            nodes: json!({}),
        };
        match node.run(&ctx, &input).unwrap() {
            Outcome::Next { port, value } => {
                assert_eq!(port.as_str(), "err");
                assert!(value["error"].as_str().unwrap().contains("not configured"));
            }
            other => panic!("{other:?}"),
        }
    }
}
