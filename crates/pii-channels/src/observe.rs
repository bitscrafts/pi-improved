//! The observability seam of the channels: [`ObservedChannel`] decorates any
//! [`Channel`], reporting poll and send failures and a cursor-lag gauge.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_observe::{Event, Level, Measure, Observer, names};

use crate::{Channel, Incoming};

/// A [`Channel`] that reports to an [`Observer`]. Message text, users and
/// credentials are never reported — only the channel name/kind and the error.
pub struct ObservedChannel {
    inner: Arc<dyn Channel>,
    observer: Arc<dyn Observer>,
}

impl ObservedChannel {
    /// Wraps `inner`, reporting to `observer`.
    #[must_use]
    pub fn new(inner: Arc<dyn Channel>, observer: Arc<dyn Observer>) -> Self {
        Self { inner, observer }
    }

    fn failure(&self, kind: &'static str, e: &PiiError) {
        let specific = if crate::is_auth_rejected(e) {
            Some(names::CHANNEL_AUTH_REJECTED)
        } else if matches!(e, PiiError::Denied(_)) {
            Some(names::CHANNEL_EGRESS_DENIED)
        } else {
            None
        };
        if let Some(name) = specific {
            self.report(name, e);
        }
        self.report(kind, e);
    }

    fn report(&self, kind: &'static str, e: &PiiError) {
        self.observer.event(
            &Event::new("channel", Level::Warn, kind)
                .field("channel", self.inner.name())
                .field("kind", self.inner.kind())
                .field("error", e.to_string()),
        );
    }
}

impl Channel for ObservedChannel {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn kind(&self) -> &str {
        self.inner.kind()
    }

    /// Polls the inner channel. A failure is [`names::CHANNEL_POLL_FAILED`];
    /// a success records the cursor lag ([`names::CHANNEL_CURSOR_LAG`]): the
    /// number of messages that were waiting past `cursor`.
    fn health_issue(&self) -> Option<String> {
        self.inner.health_issue()
    }

    fn poll(&self, ctx: &Ctx, cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        let result = self.inner.poll(ctx, cursor);
        match &result {
            Ok(msgs) => self.observer.measure(&Measure::Gauge(
                i64::try_from(msgs.len()).unwrap_or(i64::MAX),
            )),
            Err(e) => self.failure(names::CHANNEL_POLL_FAILED, e),
        }
        result
    }

    fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
        let result = self.inner.send(ctx, to, text);
        if let Err(e) = &result {
            self.failure(names::CHANNEL_SEND_FAILED, e);
        }
        result
    }
}
