//! The `mqtt` tool: the model's door to the house's devices, a sibling of `bash` and `fs_read`.
//!
//! Measured 2026-11-02 on the real installations: two Tasmota hosts (`SW-FAN-TV` at 10.10.10.254, the `zb-gw03`
//! Zigbee bridge at 10.10.10.223) and one Zigbee sensor behind the bridge. They announce themselves on retained
//! `tasmota/discovery/<mac>/config` topics and report liveness on `tele/<base>/LWT`, so listing devices is a read of
//! the broker, not a config file pii has to be told about.
//!
//! Verbs, and nothing else: a tool, not a scripting language.
//!   ls                      devices, from their own discovery messages (name, address, module, firmware, online)
//!   sub <filter> [seconds]  what a filter carries, for a bounded window
//!   pub <topic> <payload>   publish, with qos/retain explicit
//!   send <device> <command> address a device by NAME: `send SW-FAN-TV Power TOGGLE` → `cmnd/<base>/Power TOGGLE`
//!
//! The transport is a seam ([`MqttOps`]) so the whole tool is testable without a broker; the live implementation
//! wraps the channel the daemon already runs (`pii_channels::mqtt::MqttChannel`).

use std::sync::Arc;
use std::time::Duration;

use pii_abi::{CapKind, PiiError};
use pii_core::Ctx;
use pii_model::ToolSpec;
use serde_json::{Value, json};

use crate::Tool;

mod live;
pub use live::LiveOps;

/// How long a publish holds the connection open so the driver can flush it before the socket closes.
pub const FLUSH_GRACE_MS: u64 = 400;

/// How long a listing waits for retained discovery messages to arrive.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(3);

/// What the tool needs from a broker. Two operations, both bounded.
pub trait MqttOps: Send + Sync {
    /// Publishes one message. `qos` 0..=2, `retain` explicit.
    ///
    /// # Errors
    /// Whatever the transport refuses: an unreachable broker, a refused publish, a bad topic.
    fn publish(&self, topic: &str, payload: &str, qos: u8, retain: bool) -> Result<(), PiiError>;
    /// Collects what `filters` carry for `window`.
    ///
    /// # Errors
    /// Whatever the transport refuses when it cannot subscribe or read.
    fn collect(
        &self,
        filters: &[String],
        window: Duration,
    ) -> Result<Vec<(String, Vec<u8>)>, PiiError>;
}

/// A device as the broker describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The topic base a command goes to (`tasmota_C76EA4`, from the discovery mac).
    pub base: String,
    /// The friendly name the device reports (`dn`), which is what a person types.
    pub name: String,
    /// The address the device reports for itself.
    pub ip: String,
    /// The hardware module it says it is.
    pub module: String,
    /// Its firmware version.
    pub firmware: String,
    /// Last-will state, when a `tele/<base>/LWT` line was seen in the window; `None` means "not heard".
    pub online: Option<bool>,
}

/// What the model may do with the tool (spec 336). Absent on the operator's CLI, which keeps every verb.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Scope {
    /// Devices the model may `send` to, by name or base. Empty means none is writable.
    pub allow_devices: Vec<String>,
    /// Topic filters `sub` is restricted to. Empty means any filter.
    pub topics: Vec<String>,
}

/// The `mqtt` tool.
pub struct MqttTool {
    ops: Arc<dyn MqttOps>,
    /// How long a listing waits for retained messages to arrive.
    window: Duration,
    /// `Some` for the model (raw `pub` refused, `send` gated by the allowlist); `None` for the operator.
    scope: Option<Scope>,
}

/// The capability a refusal names when the context lacks it.
fn require(ctx: &Ctx, kind: CapKind, name: &str) -> Result<(), PiiError> {
    if ctx.caps().kinds.contains(kind) {
        Ok(())
    } else {
        Err(PiiError::Denied(name.to_owned()))
    }
}

impl MqttTool {
    /// The model's tool over `ops`: nothing is writable until [`MqttTool::with_allow_devices`] names a device.
    #[must_use]
    pub fn new(ops: Arc<dyn MqttOps>) -> Self {
        Self {
            ops,
            window: DEFAULT_WINDOW,
            scope: Some(Scope::default()),
        }
    }

    /// The operator's tool over `ops`: every verb, no allowlist. Only the CLI holds this.
    #[must_use]
    pub fn operator(ops: Arc<dyn MqttOps>) -> Self {
        Self {
            ops,
            window: DEFAULT_WINDOW,
            scope: None,
        }
    }

    /// Devices the model may `send` to (`[[mqtt]] allow_devices`).
    #[must_use]
    pub fn with_allow_devices(mut self, allow: Vec<String>) -> Self {
        self.scope.get_or_insert_with(Scope::default).allow_devices = allow;
        self
    }

    /// Topic filters the model's `sub` is restricted to (`[[mqtt]] topics`).
    #[must_use]
    pub fn with_topics(mut self, topics: Vec<String>) -> Self {
        self.scope.get_or_insert_with(Scope::default).topics = topics;
        self
    }

    fn devices(&self) -> Result<Vec<Device>, PiiError> {
        let got = self.ops.collect(
            &[
                String::from("tasmota/discovery/#"),
                String::from("tele/+/LWT"),
            ],
            self.window,
        )?;
        Ok(parse_devices(&got))
    }

    /// The model's `sub` is restricted to the configured topics when that list is non-empty (spec 336).
    fn may_read(&self, filter: &str) -> Result<(), PiiError> {
        match &self.scope {
            Some(scope)
                if !scope.topics.is_empty() && !scope.topics.iter().any(|t| t == filter) =>
            {
                Err(PiiError::Denied(format!(
                    "mqtt: filter {filter:?} is outside the configured topics {:?}",
                    scope.topics
                )))
            }
            _ => Ok(()),
        }
    }

    /// The model never publishes a raw topic: `pub` and its `retain` are the operator's alone (spec 336).
    fn may_publish_raw(&self) -> Result<(), PiiError> {
        match self.scope {
            Some(_) => Err(PiiError::Denied(String::from(
                "mqtt: raw publish is not available to the model; use `send` with an allowlisted device",
            ))),
            None => Ok(()),
        }
    }

    /// The model's `send` reaches only an allowlisted device, by name or base; an empty list writes nothing.
    fn may_write(&self, device: &Device, known: &[&str]) -> Result<(), PiiError> {
        let Some(scope) = &self.scope else {
            return Ok(());
        };
        let listed = scope
            .allow_devices
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&device.name) || a.eq_ignore_ascii_case(&device.base));
        if listed {
            return Ok(());
        }
        Err(PiiError::Denied(format!(
            "mqtt: {:?} is not writable for the model; allow_devices: {:?}; known: {known:?}",
            device.name, scope.allow_devices
        )))
    }
}

/// Turns discovery configs and LWT lines into devices. Split out so it is tested on real payloads with no broker.
#[must_use]
pub fn parse_devices(messages: &[(String, Vec<u8>)]) -> Vec<Device> {
    let mut by_base: std::collections::BTreeMap<String, Device> = std::collections::BTreeMap::new();
    for (topic, payload) in messages {
        if topic.starts_with("tasmota/discovery/") && topic.ends_with("/config") {
            let Some(mac) = topic.split('/').nth(2) else {
                continue;
            };
            let base = format!("tasmota_{}", &mac[mac.len().saturating_sub(6)..]);
            let Ok(v) = serde_json::from_slice::<Value>(payload) else {
                continue;
            };
            let name = v
                .get("dn")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    v.get("fn")
                        .and_then(Value::as_array)
                        .and_then(|a| a.iter().find_map(Value::as_str))
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| base.clone());
            by_base.insert(
                base.clone(),
                Device {
                    base,
                    name,
                    ip: v
                        .get("ip")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    module: v
                        .get("md")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    firmware: v
                        .get("sw")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    online: None,
                },
            );
        }
    }
    for (topic, payload) in messages {
        if let Some(base) = topic
            .strip_prefix("tele/")
            .and_then(|rest| rest.strip_suffix("/LWT"))
        {
            let state = String::from_utf8_lossy(payload).trim().to_owned();
            if let Some(d) = by_base.get_mut(base) {
                d.online = Some(state.eq_ignore_ascii_case("online"));
            }
        }
    }
    by_base.into_values().collect()
}

/// Splits a Tasmota command into the (topic, payload) pair its protocol actually uses.
///
/// MEASURED 2026-11-02 against SW-FAN-TV: publishing `cmnd/<base>/Power ON` is answered with
/// `{"Command":"Unknown","Input":"POWER ON"}` — the command is the topic's LAST SEGMENT and its argument is the
/// PAYLOAD. So `Power ON` becomes topic `cmnd/<base>/Power`, payload `ON`; a bare `Status` keeps an empty payload.
#[must_use]
pub fn command_parts(device: &Device, command: &str) -> (String, String) {
    let mut it = command.trim().splitn(2, char::is_whitespace);
    let verb = it.next().unwrap_or("").trim();
    let args = it.next().unwrap_or("").trim();
    (format!("cmnd/{}/{}", device.base, verb), args.to_owned())
}

fn find<'a>(devices: &'a [Device], wanted: &str) -> Option<&'a Device> {
    devices
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(wanted) || d.base.eq_ignore_ascii_case(wanted))
}

impl Tool for MqttTool {
    fn name(&self) -> &'static str {
        "mqtt"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("mqtt"),
            description: String::from(
                "Talk to the house's MQTT devices. op=ls lists devices from their own discovery messages; \
                 op=sub reads a topic filter for a bounded window; op=pub publishes with explicit qos/retain; \
                 op=send addresses a device by name (send SW-FAN-TV \"Power TOGGLE\").",
            ),
            schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["ls", "sub", "pub", "send"] },
                    "filter": { "type": "string", "description": "op=sub: an MQTT filter, e.g. tele/+/STATE" },
                    "seconds": { "type": "integer", "description": "op=sub: how long to listen (default 3, max 30)" },
                    "topic": { "type": "string", "description": "op=pub" },
                    "payload": { "type": "string", "description": "op=pub / op=send: the text to send" },
                    "qos": { "type": "integer", "description": "op=pub: 0, 1 or 2 (default 0)" },
                    "retain": { "type": "boolean", "description": "op=pub: default false" },
                    "device": { "type": "string", "description": "op=send: the device's name, e.g. SW-FAN-TV" }
                },
                "required": ["op"]
            }),
        }
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let op = args
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| PiiError::Invalid(String::from("mqtt: `op` is required")))?;
        match op {
            "ls" => {
                require(ctx, CapKind::MQTT_READ, "mqtt.read")?;
                let devices = self.devices()?;
                Ok(json!({
                    "count": devices.len(),
                    "devices": devices.iter().map(|d| json!({
                        "name": d.name, "base": d.base, "ip": d.ip,
                        "module": d.module, "firmware": d.firmware, "online": d.online
                    })).collect::<Vec<_>>()
                }))
            }
            "sub" => {
                require(ctx, CapKind::MQTT_READ, "mqtt.read")?;
                let filter = args.get("filter").and_then(Value::as_str).ok_or_else(|| {
                    PiiError::Invalid(String::from("mqtt: `filter` is required for sub"))
                })?;
                self.may_read(filter)?;
                let secs = args
                    .get("seconds")
                    .and_then(Value::as_u64)
                    .unwrap_or(3)
                    .clamp(1, 30);
                let got = self
                    .ops
                    .collect(&[filter.to_owned()], Duration::from_secs(secs))?;
                Ok(json!({
                    "count": got.len(),
                    "messages": got.iter().map(|(t, p)| json!({
                        "topic": t, "bytes": p.len(), "payload": String::from_utf8_lossy(p)
                    })).collect::<Vec<_>>()
                }))
            }
            "pub" => {
                self.may_publish_raw()?;
                require(ctx, CapKind::MQTT_WRITE, "mqtt.write")?;
                let topic = args.get("topic").and_then(Value::as_str).ok_or_else(|| {
                    PiiError::Invalid(String::from("mqtt: `topic` is required for pub"))
                })?;
                let payload = args.get("payload").and_then(Value::as_str).unwrap_or("");
                let qos = u8::try_from(args.get("qos").and_then(Value::as_u64).unwrap_or(0))
                    .map_err(|_| PiiError::Invalid(String::from("mqtt: qos must be 0, 1 or 2")))?;
                let retain = args.get("retain").and_then(Value::as_bool).unwrap_or(false);
                self.ops.publish(topic, payload, qos, retain)?;
                Ok(
                    json!({ "published": topic, "bytes": payload.len(), "qos": qos, "retain": retain }),
                )
            }
            "send" => {
                require(ctx, CapKind::MQTT_WRITE, "mqtt.write")?;
                let name = args.get("device").and_then(Value::as_str).ok_or_else(|| {
                    PiiError::Invalid(String::from("mqtt: `device` is required for send"))
                })?;
                let command = args.get("payload").and_then(Value::as_str).unwrap_or("");
                if command.trim().is_empty() {
                    return Err(PiiError::Invalid(String::from(
                        "mqtt: `payload` is the command to send",
                    )));
                }
                let devices = self.devices()?;
                let known: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
                let device = find(&devices, name).ok_or_else(|| {
                    PiiError::Invalid(format!("mqtt: no device named {name:?}; known: {known:?}"))
                })?;
                self.may_write(device, &known)?;
                let (topic, payload) = command_parts(device, command);
                self.ops.publish(&topic, &payload, 0, false)?;
                Ok(
                    json!({ "device": device.name, "topic": topic, "payload": payload, "command": command }),
                )
            }
            other => Err(PiiError::Invalid(format!("mqtt: unknown op {other:?}"))),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use pii_core::{Budget, CapSet};
    use std::sync::mpsc::{Receiver, Sender, channel};

    /// REAL payload shapes, copied from the live broker on 2026-11-02: a discovery config for the Zigbee bridge
    /// and one for the Sonoff, plus their LWT lines.
    fn discovery() -> Vec<(String, Vec<u8>)> {
        vec![
            (
                String::from("tasmota/discovery/2043A8C76EA4/config"),
                br#"{"ip":"10.10.10.223","dn":"zb-gw03","fn":["zg-gw03",null,null,null,null],"md":"ZB-GW03","sw":"15.2.0.4"}"#.to_vec(),
            ),
            (
                String::from("tasmota/discovery/94A9906DE85C/config"),
                br#"{"ip":"10.10.10.254","dn":"SW-FAN-TV","fn":["SW-FAN-TV"],"md":"Sonoff BasicR4","sw":"15.3.0"}"#.to_vec(),
            ),
            (
                String::from("tele/tasmota_C76EA4/LWT"),
                b"Online".to_vec(),
            ),
        ]
    }

    #[test]
    fn devices_are_read_from_their_own_discovery_messages() {
        let devices = parse_devices(&discovery());
        assert_eq!(devices.len(), 2, "{devices:?}");
        let sonoff = devices.iter().find(|d| d.name == "SW-FAN-TV").unwrap();
        assert_eq!(sonoff.base, "tasmota_6DE85C");
        assert_eq!(sonoff.ip, "10.10.10.254");
        assert_eq!(sonoff.module, "Sonoff BasicR4");
        assert_eq!(sonoff.firmware, "15.3.0");
        // the LWT line names the OTHER device's base, so this one stays unknown — not assumed online
        assert_eq!(sonoff.online, None);
        let bridge = devices.iter().find(|d| d.name == "zb-gw03").unwrap();
        assert_eq!(bridge.online, Some(true));
    }

    #[test]
    fn a_command_is_addressed_by_name_to_its_base_topic() {
        let devices = parse_devices(&discovery());
        let sonoff = devices.iter().find(|d| d.name == "SW-FAN-TV").unwrap();
        assert_eq!(
            command_parts(sonoff, "Power TOGGLE"),
            (
                String::from("cmnd/tasmota_6DE85C/Power"),
                String::from("TOGGLE")
            )
        );
    }

    #[test]
    fn a_missing_name_is_refused_and_the_known_ones_are_named() {
        let (ops, _rx) = fakes(discovery());
        let tool = MqttTool::operator(ops);
        let ctx = Ctx::root(
            CapSet {
                kinds: CapKind::MQTT_READ | CapKind::MQTT_WRITE,
                ..CapSet::default()
            },
            Budget::unlimited(),
        );
        let err = tool
            .call(
                &ctx,
                &json!({"op": "send", "device": "NOPE", "payload": "Power ON"}),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("no device named"), "{err}");
        assert!(
            err.contains("SW-FAN-TV"),
            "the refusal names what exists: {err}"
        );
    }

    #[test]
    fn send_publishes_to_the_device_topic_and_ls_lists_them() {
        let (ops, rx) = fakes(discovery());
        let tool = MqttTool::operator(ops);
        let ctx = Ctx::root(
            CapSet {
                kinds: CapKind::MQTT_READ | CapKind::MQTT_WRITE,
                ..CapSet::default()
            },
            Budget::unlimited(),
        );

        let listed = tool.call(&ctx, &json!({"op": "ls"})).unwrap();
        assert_eq!(listed["count"], 2, "{listed}");

        let sent = tool
            .call(
                &ctx,
                &json!({"op": "send", "device": "SW-FAN-TV", "payload": "Power TOGGLE"}),
            )
            .unwrap();
        assert_eq!(sent["topic"], "cmnd/tasmota_6DE85C/Power");
        assert_eq!(sent["payload"], "TOGGLE");
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, "cmnd/tasmota_6DE85C/Power");
        assert_eq!(got[0].1, "TOGGLE");
    }

    #[test]
    fn a_bad_op_is_refused_rather_than_ignored() {
        let (ops, _rx) = fakes(vec![]);
        let tool = MqttTool::operator(ops);
        let err = tool
            .call(
                &Ctx::root(CapSet::empty(), Budget::unlimited()),
                &json!({"op": "reboot_everything"}),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown op"), "{err}");
    }

    /// One recorded publish: `(topic, payload, qos, retain)`.
    type Published = (String, String, u8, bool);

    /// Records what was published through a CHANNEL, not a lock: `std::sync::Mutex` is a disallowed type in this
    /// crate, and a sender is already `Sync`.
    struct FakeOps {
        messages: Vec<(String, Vec<u8>)>,
        published: Sender<Published>,
    }

    fn fakes(messages: Vec<(String, Vec<u8>)>) -> (Arc<FakeOps>, Receiver<Published>) {
        let (tx, rx) = channel();
        (
            Arc::new(FakeOps {
                messages,
                published: tx,
            }),
            rx,
        )
    }

    impl MqttOps for FakeOps {
        fn publish(
            &self,
            topic: &str,
            payload: &str,
            qos: u8,
            retain: bool,
        ) -> Result<(), PiiError> {
            let _ = self
                .published
                .send((topic.to_owned(), payload.to_owned(), qos, retain));
            Ok(())
        }
        fn collect(
            &self,
            _filters: &[String],
            _window: Duration,
        ) -> Result<Vec<(String, Vec<u8>)>, PiiError> {
            Ok(self.messages.clone())
        }
    }
}
