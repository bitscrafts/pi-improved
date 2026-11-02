//! `pii mqtt <verb>` — the operator's door to the house's devices, the same tool the model is given.
//!
//! Verbs, and nothing else:
//!   pii mqtt ls                      devices, from their own discovery messages
//!   pii mqtt sub <filter> [secs]     what a filter carries, for a bounded window
//!   pii mqtt pub <topic> <text>      publish; --qos 0|1|2, --retain
//!   pii mqtt send <device> <command> address a device by NAME
//!
//! The broker comes from `--url`, else `PII_MQTT_URL`, else `mqtt://127.0.0.1:1883`. Output is one JSON object per
//! call, the same shape the tool returns, so a script and the model see the same thing.

use std::process::ExitCode;
use std::sync::Arc;

use pii_core::{Budget, CapSet, Ctx};
use pii_tools::Tool;
use pii_tools::mqtt::{LiveOps, MqttTool};

use crate::cmd_ctl::next_str;

fn usage() -> ExitCode {
    eprintln!(
        "usage: pii mqtt <ls|sub|pub|send> [args] [--url mqtt://host:port]\n  \
         ls                      list devices from their discovery messages\n  \
         sub <filter> [secs]     read a filter for a bounded window\n  \
         pub <topic> <text>      publish (--qos 0|1|2, --retain)\n  \
         send <device> <command> address a device by name, e.g. send SW-FAN-TV \"Power TOGGLE\""
    );
    ExitCode::from(2)
}

/// `pii mqtt …`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_mqtt(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut verb = None;
    let mut positional: Vec<String> = Vec::new();
    let mut url: Option<String> = None;
    let mut qos: u64 = 0;
    let mut retain = false;
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--url" => url = next_str(args),
            "--qos" => qos = next_str(args).and_then(|s| s.parse().ok()).unwrap_or(0),
            "--retain" => retain = true,
            s if s.starts_with("--") => return usage(),
            s if verb.is_none() => verb = Some(s.to_owned()),
            s => positional.push(s.to_owned()),
        }
    }
    let Some(verb) = verb else { return usage() };
    let url = url
        .or_else(|| pii_core::env_var("PII_MQTT_URL"))
        .unwrap_or_else(|| String::from("mqtt://127.0.0.1:1883"));

    let observer: Arc<dyn pii_observe::Observer> = Arc::new(pii_observe::NullObserver);
    let tool = MqttTool::new(Arc::new(LiveOps::new(url, observer)));
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());

    let args = match verb.as_str() {
        "ls" => serde_json::json!({ "op": "ls" }),
        "sub" => {
            let Some(filter) = positional.first() else {
                return usage();
            };
            let secs = positional
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            serde_json::json!({ "op": "sub", "filter": filter, "seconds": secs })
        }
        "pub" => {
            let (Some(topic), Some(text)) = (positional.first(), positional.get(1)) else {
                return usage();
            };
            serde_json::json!({ "op": "pub", "topic": topic, "payload": text, "qos": qos, "retain": retain })
        }
        "send" => {
            let (Some(device), Some(command)) = (positional.first(), positional.get(1)) else {
                return usage();
            };
            serde_json::json!({ "op": "send", "device": device, "payload": command })
        }
        _ => return usage(),
    };

    match tool.call(&ctx, &args) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pii mqtt: {e}");
            ExitCode::FAILURE
        }
    }
}
