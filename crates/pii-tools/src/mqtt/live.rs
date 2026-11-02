//! The live broker transport for the `mqtt` tool: a connection per call, opened and dropped (spec 336 split).

use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_core::{Budget, CapSet, Ctx};

use super::{FLUSH_GRACE_MS, MqttOps};

/// A live transport that opens the broker connection FOR THE CALL and closes it again.
///
/// A standing `#` subscription would double every message the daemon's own channel already carries, for a tool the
/// model uses occasionally. So: start, act, drop — the channel's `Drop` stops the driver thread.
pub struct LiveOps {
    url: String,
    observer: Arc<dyn pii_observe::Observer>,
}

impl LiveOps {
    /// A transport that opens and closes a connection per call.
    #[must_use]
    pub fn new(url: impl Into<String>, observer: Arc<dyn pii_observe::Observer>) -> Self {
        Self {
            url: url.into(),
            observer,
        }
    }

    fn channel(&self, topics: &[String]) -> Result<Arc<pii_channels::mqtt::MqttChannel>, PiiError> {
        let mut cfg = pii_channels::mqtt::MqttConfig::new("tool", &self.url);
        cfg.client_id = Some(format!("pii-tool-{}", std::process::id()));
        cfg.topics = topics.to_vec();
        pii_channels::mqtt::MqttChannel::start(cfg, Arc::clone(&self.observer))
    }
}

impl MqttOps for LiveOps {
    fn publish(&self, topic: &str, payload: &str, qos: u8, retain: bool) -> Result<(), PiiError> {
        let ch = self.channel(&[])?;
        ch.publish(topic, payload.as_bytes(), qos, retain)?;
        // MEASURED 2026-11-02: the channel hands the message to a driver thread and dropping the channel closes the
        // socket, so a command published and immediately dropped never reaches the broker — the round trip got zero
        // replies. The connection is held briefly while the driver flushes; QoS 1 alone would only mean "queued".
        std::thread::sleep(Duration::from_millis(FLUSH_GRACE_MS));
        Ok(())
    }

    fn collect(
        &self,
        filters: &[String],
        window: Duration,
    ) -> Result<Vec<(String, Vec<u8>)>, PiiError> {
        use pii_channels::Channel;
        let ch = self.channel(filters)?;
        let deadline = Instant::now() + window;
        let mut out = Vec::new();
        while Instant::now() < deadline {
            let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
            for msg in ch.poll(&ctx, None)? {
                out.push((msg.chat.clone(), msg.text.into_bytes()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(out)
    }
}
