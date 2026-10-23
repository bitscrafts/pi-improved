//! Spec 101 R3: poll failures, send failures and the cursor-lag measure reach
//! a `pii-observe` observer by their registered names.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::PiiError;
use pii_channels::observe::ObservedChannel;
use pii_channels::{Channel, Incoming};
use pii_core::{Budget, CapSet, Ctx};
use pii_observe::{Measure, Recorder, names};

struct Scripted {
    fail: bool,
}

impl Channel for Scripted {
    fn name(&self) -> &'static str {
        "c1"
    }
    fn kind(&self) -> &'static str {
        "scripted"
    }
    fn poll(&self, _ctx: &Ctx, _cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        if self.fail {
            return Err(PiiError::Unavailable(String::from("boom")));
        }
        Ok((1..=3)
            .map(|n| Incoming {
                channel: String::from("c1"),
                chat: String::from("x"),
                user: String::from("u"),
                text: String::from("t"),
                cursor: n,
            })
            .collect())
    }
    fn send(&self, _ctx: &Ctx, _to: &str, _text: &str) -> Result<(), PiiError> {
        if self.fail {
            return Err(PiiError::Unavailable(String::from("nope")));
        }
        Ok(())
    }
}

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

#[test]
fn poll_and_send_failures_are_reported_by_name() {
    let rec = Arc::new(Recorder::new());
    let ch = ObservedChannel::new(Arc::new(Scripted { fail: true }), rec.clone());
    assert!(ch.poll(&ctx(), None).is_err());
    assert!(ch.send(&ctx(), "x", "secret text").is_err());
    let polls = rec.events_named(names::CHANNEL_POLL_FAILED);
    assert_eq!(polls.len(), 1, "{:?}", rec.events());
    assert_eq!(
        polls[0].fields.get("channel"),
        Some(&serde_json::json!("c1"))
    );
    let sends = rec.events_named(names::CHANNEL_SEND_FAILED);
    assert_eq!(sends.len(), 1);
    // The message text is never reported.
    assert!(!format!("{:?}", rec.events()).contains("secret text"));
    assert!(rec.measures().is_empty(), "no lag measure on a failed poll");
}

#[test]
fn successful_poll_records_cursor_lag_and_is_otherwise_silent() {
    let rec = Arc::new(Recorder::new());
    let ch = ObservedChannel::new(Arc::new(Scripted { fail: false }), rec.clone());
    assert_eq!(ch.poll(&ctx(), Some(0)).unwrap().len(), 3);
    assert_eq!(rec.measures(), vec![Measure::Gauge(3)]);
    assert_eq!(rec.events(), [] as [pii_observe::OwnedEvent; 0]);
    assert_eq!(names::CHANNEL_CURSOR_LAG, "channel.cursor.lag");
}
