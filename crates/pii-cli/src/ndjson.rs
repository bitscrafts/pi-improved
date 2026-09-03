//! Ndjson stdout observer (spec F5.5).
//!
//! Emits one ndjson line per engine observation: `{"metric":<name>,"value":<u64>}`
//! for metrics and `{"seq":<u64>,"event":<RunEvent>}` for journal events. The
//! final `{"terminal":..}` line is printed by the run/resume command itself,
//! after the engine returns.

use pii_core::{Metric, ObsEvent, Observer};

/// Observer that streams the run as ndjson lines on stdout.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) struct NdjsonObserver;

impl Observer for NdjsonObserver {
    fn event(&self, e: &ObsEvent) {
        if let ObsEvent::Journal { seq, event } = e {
            match serde_json::to_string(event) {
                Ok(body) => println!(r#"{{"seq":{seq},"event":{body}}}"#),
                Err(err) => {
                    let msg = err.to_string().replace('\\', "\\\\").replace('"', "'");
                    println!(r#"{{"seq":{seq},"event":{{"serialize_error":"{msg}"}}}}"#);
                }
            }
        }
    }

    fn metric(&self, m: &Metric) {
        // `Metric` is not `serde::Serialize`; render the F5.5 shape by hand.
        println!(
            "{}",
            serde_json::json!({"metric": m.name, "value": m.value})
        );
    }

    fn flush(&self) {}
}
