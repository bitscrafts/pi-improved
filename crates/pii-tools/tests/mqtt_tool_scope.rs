//! Spec 336: the model's `mqtt` tool obeys a device allowlist and its capability set.
//!
//! No broker: the fake [`MqttOps`] sends every publish down a channel and answers discovery with the real payloads
//! measured on 2026-11-02.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use pii_abi::{CapKind, PiiError};
use pii_core::{Budget, CapSet, Ctx};
use pii_tools::Tool;
use pii_tools::mqtt::{MqttOps, MqttTool};
use serde_json::json;

/// One recorded publish: `(topic, payload, qos, retain)`.
type Publish = (String, String, u8, bool);

/// Records publishes through a channel, not a lock: a sender is already `Sync`.
struct Fake {
    published: Sender<Publish>,
}

/// The fake and the receiver its publishes arrive on.
fn recorder() -> (Arc<Fake>, Receiver<Publish>) {
    let (tx, rx) = channel();
    (Arc::new(Fake { published: tx }), rx)
}

fn published(rx: &Receiver<Publish>) -> Vec<Publish> {
    rx.try_iter().collect()
}

impl MqttOps for Fake {
    fn publish(&self, topic: &str, payload: &str, qos: u8, retain: bool) -> Result<(), PiiError> {
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
        Ok(vec![
            (
                String::from("tasmota/discovery/2043A8C76EA4/config"),
                br#"{"ip":"10.10.10.223","dn":"zb-gw03","fn":["zg-gw03",null,null,null,null],"md":"ZB-GW03","sw":"15.2.0.4"}"#.to_vec(),
            ),
            (
                String::from("tasmota/discovery/94A9906DE85C/config"),
                br#"{"ip":"10.10.10.254","dn":"SW-FAN-TV","fn":["SW-FAN-TV"],"md":"Sonoff BasicR4","sw":"15.3.0"}"#.to_vec(),
            ),
        ])
    }
}

/// A context holding both mqtt capabilities, as a granted model run and the operator's CLI hold them.
fn full_ctx() -> Ctx {
    Ctx::root(
        CapSet {
            kinds: CapKind::MQTT_READ | CapKind::MQTT_WRITE,
            ..CapSet::default()
        },
        Budget::unlimited(),
    )
}

/// A context holding only `mqtt.read`.
fn read_only_ctx() -> Ctx {
    Ctx::root(
        CapSet {
            kinds: CapKind::MQTT_READ,
            ..CapSet::default()
        },
        Budget::unlimited(),
    )
}

fn tool(fake: &Arc<Fake>, allow: &[&str]) -> MqttTool {
    MqttTool::new(Arc::clone(fake) as Arc<dyn MqttOps>)
        .with_allow_devices(allow.iter().map(|s| (*s).to_owned()).collect())
}

fn message_of(err: &PiiError) -> String {
    match err {
        PiiError::Denied(m) | PiiError::Invalid(m) => m.clone(),
        other => format!("{other:?}"),
    }
}

#[test]
fn pub_is_refused_for_the_model() {
    let (fake, rx) = recorder();
    let args = json!({"op": "pub", "topic": "cmnd/x/Power", "payload": "ON"});
    let err = tool(&fake, &["SW-FAN-TV"])
        .call(&full_ctx(), &args)
        .unwrap_err();
    assert!(matches!(err, PiiError::Denied(_)), "{err:?}");
    assert!(message_of(&err).contains("raw publish"));
    assert_eq!(published(&rx), Vec::new());
}

#[test]
fn send_to_an_unlisted_device_is_refused_and_names_the_allowlist() {
    let (fake, rx) = recorder();
    let args = json!({"op": "send", "device": "zb-gw03", "payload": "Status 0"});
    let err = tool(&fake, &["SW-FAN-TV"])
        .call(&full_ctx(), &args)
        .unwrap_err();
    assert!(message_of(&err).contains("SW-FAN-TV"), "{err:?}");
    assert_eq!(published(&rx), Vec::new());
}

#[test]
fn send_to_a_listed_device_publishes_without_retain() {
    let (fake, rx) = recorder();
    let args = json!({"op": "send", "device": "SW-FAN-TV", "payload": "Power ON"});
    tool(&fake, &["SW-FAN-TV"])
        .call(&full_ctx(), &args)
        .unwrap();
    assert_eq!(
        published(&rx),
        vec![(
            String::from("cmnd/tasmota_6DE85C/Power"),
            String::from("ON"),
            0,
            false
        )]
    );
}

#[test]
fn control_an_empty_allowlist_makes_every_device_unwritable() {
    let (fake, rx) = recorder();
    let t = tool(&fake, &[]);
    let send = json!({"op": "send", "device": "SW-FAN-TV", "payload": "Power ON"});
    let err = t.call(&full_ctx(), &send).unwrap_err();
    assert!(matches!(err, PiiError::Denied(_)), "{err:?}");
    assert_eq!(published(&rx), Vec::new());

    let listed = t.call(&full_ctx(), &json!({"op": "ls"})).unwrap();
    assert_eq!(listed["count"], 2, "{listed}");
}

#[test]
fn ls_needs_mqtt_read() {
    let (fake, _rx) = recorder();
    let empty = Ctx::root(CapSet::empty(), Budget::unlimited());
    let err = tool(&fake, &["SW-FAN-TV"])
        .call(&empty, &json!({"op": "ls"}))
        .unwrap_err();
    assert!(matches!(err, PiiError::Denied(_)), "{err:?}");
    assert!(message_of(&err).contains("mqtt.read"), "{err:?}");
}

#[test]
fn send_needs_mqtt_write_even_for_a_listed_device() {
    let (fake, rx) = recorder();
    let args = json!({"op": "send", "device": "SW-FAN-TV", "payload": "Power ON"});
    let err = tool(&fake, &["SW-FAN-TV"])
        .call(&read_only_ctx(), &args)
        .unwrap_err();
    assert!(message_of(&err).contains("mqtt.write"), "{err:?}");
    assert_eq!(published(&rx), Vec::new());
}
