//! Spec 117 R5: the MQTT channel against an in-process loopback broker.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod mqtt_broker;

use std::sync::Arc;
use std::time::{Duration, Instant};

use mqtt_broker::{Broker, Seen};
use pii_channels::mqtt::{MqttChannel, MqttConfig, backoff_ms};
use pii_channels::{Channel, Incoming};
use pii_core::{Budget, CapSet, Ctx};
use pii_observe::{Recorder, names};

const CANARY: &str = "CANARY-117-hunter2";

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

fn cfg(port: u16) -> MqttConfig {
    let mut c = MqttConfig::new("m", &format!("mqtt://127.0.0.1:{port}"));
    c.topics = vec!["sensors/#".into()];
    c.reconnect_min_ms = 20;
    c.reconnect_max_ms = 80;
    c
}

fn start(c: MqttConfig) -> (Arc<MqttChannel>, Arc<Recorder>) {
    let rec = Arc::new(Recorder::new());
    let ch = MqttChannel::start(c, rec.clone()).unwrap();
    (ch, rec)
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for: {what}");
}

fn poll_n(ch: &MqttChannel, n: usize) -> Vec<Incoming> {
    let mut got = Vec::new();
    until("messages", || {
        got.extend(ch.poll(&ctx(), None).unwrap());
        got.len() >= n
    });
    got
}

#[test]
fn subscribe_message_becomes_incoming_with_payload_and_topic() {
    let b = Broker::start(false);
    let (ch, rec) = start(cfg(b.port));
    until("subscribed", || !b.subscribed().is_empty());
    assert_eq!(b.subscribed(), vec!["sensors/#".to_owned()]);
    b.push("sensors/temp", br#"{"t":21}"#);
    let got = poll_n(&ch, 1);
    assert_eq!(got[0].chat, "sensors/temp");
    assert_eq!(got[0].text, r#"{"t":21}"#);
    assert_eq!(got[0].channel, "m");
    assert_eq!(ch.counters().unwrap().received, 1);
    let ev = rec.events_named(names::MQTT_MESSAGE_RECEIVED);
    assert_eq!(ev.len(), 1);
    assert!(format!("{:?}", ev[0].fields).contains("sensors/temp"));
    assert_ne!(
        rec.events_named(names::MQTT_CONNECTED),
        [] as [pii_observe::OwnedEvent; 0]
    );
    assert!(ch.health_issue().is_none());
}

#[test]
fn publish_arrives_with_topic_payload_qos_and_retain() {
    let b = Broker::start(false);
    let (ch, rec) = start(cfg(b.port));
    until("connected", || !b.connects().is_empty());
    ch.publish("out/a", b"hello", 1, true).unwrap();
    ch.publish("out/b", b"world", 0, false).unwrap();
    until("both published", || b.published().len() == 2);
    let seen = b.published();
    assert!(seen.contains(&Seen {
        topic: "out/a".into(),
        payload: b"hello".to_vec(),
        qos: 1,
        retain: true
    }));
    assert!(seen.contains(&Seen {
        topic: "out/b".into(),
        payload: b"world".to_vec(),
        qos: 0,
        retain: false
    }));
    until("counted", || ch.counters().unwrap().published == 2);
    assert_eq!(rec.events_named(names::MQTT_PUBLISHED).len(), 2);
}

#[test]
fn invalid_qos_is_refused_not_clamped() {
    let b = Broker::start(false);
    let (ch, _rec) = start(cfg(b.port));
    assert!(ch.publish("t", b"x", 3, false).is_err());
}

#[test]
fn backoff_doubles_to_a_ceiling_and_never_overflows() {
    let seq: Vec<u64> = (0..6).map(|a| backoff_ms(a, 20, 80)).collect();
    assert_eq!(seq, vec![20, 40, 80, 80, 80, 80]);
    assert_eq!(backoff_ms(u32::MAX, 20, 80), 80);
    assert_eq!(backoff_ms(0, 500, 100), 100, "ceiling wins over base");
}

#[test]
fn broker_down_is_degraded_with_reason_event_and_bounded_attempts() {
    let port = Broker::free_port(); // nothing listens here now
    let (ch, rec) = start(cfg(port));
    std::thread::sleep(Duration::from_millis(500));
    let issue = ch
        .health_issue()
        .expect("degraded while the broker is down");
    assert!(issue.contains("127.0.0.1"), "{issue}");
    let attempts = ch.reconnect_attempts();
    // delays 20,40,80,80,...: at most 1 + 7 attempts fit in 500 ms. A hot
    // loop would be in the thousands; the lower bound proves it does retry.
    assert!(
        (4..=9).contains(&attempts),
        "bounded exponential backoff: {attempts} attempts in 500ms"
    );
    let lost = rec.events_named(names::MQTT_CONNECTION_LOST);
    assert_eq!(lost.len() as u64, attempts, "one named event per failure");
    // The broker comes back on the same port: the channel heals.
    let b = Broker::start_on(port, false);
    until("healed", || ch.health_issue().is_none());
    assert_ne!(b.connects(), [] as [mqtt_broker::Connect; 0]);
    let after = ch.reconnect_attempts();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(ch.reconnect_attempts(), after, "no retries once connected");
}

#[test]
fn credentials_to_a_plaintext_non_loopback_broker_are_refused_unless_opted_in() {
    let mut c = MqttConfig::new("m", "mqtt://broker.example.invalid:1883");
    c.username = Some("u".into());
    c.password = Some(CANARY.into());
    let err = MqttChannel::start(c.clone(), Arc::new(Recorder::new()))
        .err()
        .expect("refused");
    let msg = err.to_string();
    assert!(msg.contains("plaintext"), "{msg}");
    assert!(!msg.contains(CANARY), "{msg}");
    // Opt-in: allowed, and a warning event is emitted.
    c.allow_plaintext_credentials = true;
    let rec = Arc::new(Recorder::new());
    let ch = MqttChannel::start(c, rec.clone()).unwrap();
    assert_eq!(rec.events_named(names::MQTT_PLAINTEXT_CREDENTIALS).len(), 1);
    drop(ch);
}

#[test]
fn credentials_to_a_loopback_broker_are_sent_and_never_leak() {
    let b = Broker::start(true); // rejects: the failure path is where leaks hide
    let mut c = cfg(b.port);
    c.username = Some("alice".into());
    c.password = Some(CANARY.into());
    let (ch, rec) = start(c.clone());
    until("auth refused", || {
        !rec.events_named(names::MQTT_CONNECTION_LOST).is_empty()
    });
    until("broker saw connect", || !b.connects().is_empty());
    assert_eq!(b.connects()[0].password.as_deref(), Some(CANARY));
    let all = format!(
        "{:?} {:?} {:?} {:?}",
        rec.events(),
        ch.health_issue(),
        c,
        ch.poll(&ctx(), None)
    );
    assert!(!all.contains(CANARY), "credential leaked: {all}");
    // Accepting broker, loopback + creds: connects fine.
    let b2 = Broker::start(false);
    let mut c2 = cfg(b2.port);
    c2.username = Some("alice".into());
    c2.password = Some(CANARY.into());
    let (ch2, _r) = start(c2);
    until("connected", || {
        ch2.health_issue().is_none() && !b2.connects().is_empty()
    });
}

#[test]
fn mqtts_is_refused_because_this_build_has_no_tls() {
    let c = MqttConfig::new("m", "mqtts://127.0.0.1:8883");
    let err = MqttChannel::start(c, Arc::new(Recorder::new()))
        .err()
        .unwrap();
    assert!(err.to_string().contains("TLS"), "{err}");
}

#[test]
fn malformed_payload_is_dropped_counted_reported_and_does_not_panic() {
    let b = Broker::start(false);
    let (ch, rec) = start(cfg(b.port));
    until("subscribed", || !b.subscribed().is_empty());
    b.push("sensors/x", &[0xff, 0xfe, 0x80]);
    b.push("sensors/x", b"fine");
    let got = poll_n(&ch, 1);
    assert_eq!(got[0].text, "fine");
    let c = ch.counters().unwrap();
    assert_eq!((c.received, c.dropped), (1, 1));
    let ev = rec.events_named(names::MQTT_MESSAGE_DROPPED);
    assert_eq!(ev.len(), 1);
    assert!(format!("{:?}", ev[0].fields).contains("UTF-8"));
}

#[test]
fn a_full_queue_drops_and_counts_instead_of_growing() {
    let b = Broker::start(false);
    let mut c = cfg(b.port);
    c.queue_cap = 2;
    let (ch, rec) = start(c);
    until("subscribed", || !b.subscribed().is_empty());
    for i in 0..5 {
        b.push("sensors/q", format!("m{i}").as_bytes());
    }
    until("all seen", || {
        let c = ch.counters().unwrap();
        c.received + c.dropped == 5
    });
    let c = ch.counters().unwrap();
    assert_eq!((c.received, c.dropped), (2, 3));
    assert_eq!(rec.events_named(names::MQTT_MESSAGE_DROPPED).len(), 3);
    assert_eq!(ch.poll(&ctx(), None).unwrap().len(), 2);
}

#[test]
fn default_client_ids_are_unique_per_instance() {
    let b = Broker::start(false);
    let (_a, _) = start(cfg(b.port));
    let (_c, _) = start(cfg(b.port));
    until("two connects", || b.connects().len() == 2);
    let ids: Vec<_> = b.connects().into_iter().map(|c| c.client_id).collect();
    assert_ne!(ids[0], ids[1], "{ids:?}");
    assert!(ids.iter().all(|i| i.starts_with("pii-")));
}

#[test]
fn send_without_a_reply_topic_is_a_documented_no_op() {
    let b = Broker::start(false);
    let (ch, _) = start(cfg(b.port));
    ch.send(&ctx(), "sensors/temp", "reply").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(b.published().is_empty(), "no feedback loop onto the topic");
}
