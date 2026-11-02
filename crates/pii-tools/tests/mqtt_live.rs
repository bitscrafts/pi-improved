//! The `mqtt` tool against a REAL broker and REAL devices. `#[ignore]`d: it needs the broker and the house.
//!
//! Run by hand: `PII_MQTT_URL=mqtt://10.10.10.217:1883 cargo test -p pii-tools --test mqtt_live -- --ignored`
//!
//! Measured 2026-11-02, first run: 2 devices listed (SW-FAN-TV at 10.10.10.254 `Sonoff BasicR4`, zb-gw03 at
//! 10.10.10.223 ZB-GW03), and `send SW-FAN-TV "Status 0"` published to `cmnd/tasmota_6DE85C/Status 0`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::doc_markdown,
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::disallowed_methods
)]

use std::sync::Arc;

use pii_tools::Tool;
use pii_tools::mqtt::{LiveOps, MqttOps, MqttTool};

fn observer() -> Arc<dyn pii_observe::Observer> {
    Arc::new(pii_observe::NullObserver)
}

fn ctx() -> pii_core::Ctx {
    let caps = pii_core::CapSet {
        kinds: pii_core::CapKind::MQTT_READ | pii_core::CapKind::MQTT_WRITE,
        ..pii_core::CapSet::default()
    };
    pii_core::Ctx::root(caps, pii_core::Budget::unlimited())
}

fn url() -> String {
    pii_core::env_var("PII_MQTT_URL").unwrap_or_else(|| String::from("mqtt://10.10.10.217:1883"))
}

#[test]
#[ignore = "needs a live broker: set PII_MQTT_URL"]
fn live_list_reads_the_devices_that_are_actually_there() {
    let tool = MqttTool::operator(Arc::new(LiveOps::new(url(), observer())));
    let listed = tool.call(&ctx(), &serde_json::json!({"op": "ls"})).unwrap();
    let count = listed["count"].as_u64().unwrap();
    println!("devices: {count} — {listed}");
    assert!(count >= 1, "the house has at least one device: {listed}");
    // every entry carries the base a command would go to
    for d in listed["devices"].as_array().unwrap() {
        assert!(
            d["base"]
                .as_str()
                .is_some_and(|b| b.starts_with("tasmota_")),
            "{d}"
        );
        assert!(d["name"].as_str().is_some(), "{d}");
    }
}

#[test]
#[ignore = "needs a live broker: set PII_MQTT_URL"]
fn live_send_addresses_a_device_by_name() {
    let tool = MqttTool::operator(Arc::new(LiveOps::new(url(), observer())));
    let listed = tool.call(&ctx(), &serde_json::json!({"op": "ls"})).unwrap();
    let Some(first) = listed["devices"].as_array().and_then(|a| a.first()) else {
        println!("no device to address; nothing to prove");
        return;
    };
    let name = first["name"].as_str().unwrap();
    // `Status 0` is a read-only Tasmota command: the device answers with its telemetry and changes nothing.
    let sent = tool
        .call(
            &ctx(),
            &serde_json::json!({"op": "send", "device": name, "payload": "Status 0"}),
        )
        .unwrap();
    println!("sent: {sent}");
    assert_eq!(sent["device"], name);
    assert!(
        sent["topic"].as_str().unwrap().starts_with("cmnd/tasmota_"),
        "the command goes to the device's own cmnd topic: {sent}"
    );
}

#[test]
#[ignore = "needs a live broker: set PII_MQTT_URL"]
fn live_sub_reads_a_filter() {
    let tool = MqttTool::operator(Arc::new(LiveOps::new(url(), observer())));
    let got = tool
        .call(
            &ctx(),
            &serde_json::json!({"op": "sub", "filter": "tele/+/LWT", "seconds": 3}),
        )
        .unwrap();
    println!("sub: {got}");
    // a filter that matches nothing is a legitimate answer; what must hold is the shape
    assert!(got["count"].as_u64().is_some(), "{got}");
}

#[test]
#[ignore = "needs a live broker: set PII_MQTT_URL"]
fn live_command_gets_an_answer() {
    // The round trip that "manage" means: publish a command, then hear the device's reply as an incoming message.
    // `Status 0` is read-only, so the worst case of a wrong device name is a log line on the device, not a change.
    let tool = MqttTool::operator(Arc::new(LiveOps::new(url(), observer())));
    let listed = tool.call(&ctx(), &serde_json::json!({"op": "ls"})).unwrap();
    let Some(first) = listed["devices"].as_array().and_then(|a| a.first()) else {
        println!("no device to address");
        return;
    };
    let name = first["name"].as_str().unwrap().to_owned();
    let base = first["base"].as_str().unwrap().to_owned();

    let ops = Arc::new(LiveOps::new(url(), observer()));
    let watcher = Arc::clone(&ops);
    let filter_base = base.clone();
    let handle = std::thread::spawn(move || {
        let filter = format!("stat/{filter_base}/#");
        watcher.collect(&[filter], std::time::Duration::from_secs(6))
    });
    std::thread::sleep(std::time::Duration::from_millis(800)); // let the subscription land first
    tool.call(
        &ctx(),
        &serde_json::json!({"op": "send", "device": name, "payload": "Status 0"}),
    )
    .unwrap();
    let got = handle.join().unwrap().unwrap();
    println!("reply on stat/{base}/#: {} message(s)", got.len());
    for (t, p) in got.iter().take(3) {
        println!(
            "   {t}: {}",
            String::from_utf8_lossy(p)
                .chars()
                .take(90)
                .collect::<String>()
        );
    }
    assert!(!got.is_empty(), "the device answered a Status command");
}
