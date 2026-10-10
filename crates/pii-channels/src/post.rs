//! The egress seam of the `post` node (spec 128).
//!
//! [`ChannelHub`] holds the daemon's configured channels and sends through `Channel::send` — the
//! one place a graph can write into a channel. No second client: whatever the channel does on
//! the wire, it does here too.
//!
//! A post is outward-facing, so the hub bounds it: a per-call timeout (a hung channel never
//! hangs a run), a bounded error message, and an idempotence key (a retried step that already
//! delivered returns `Duplicate` instead of posting twice).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_core::Ctx;

use crate::Channel;

/// Longest error text the hub passes on (a provider body can be large).
pub const MAX_ERROR_BYTES: usize = 600;
/// Idempotence keys remembered before the oldest is forgotten.
pub const KEY_LOG_CAP: usize = 4096;

struct Slot {
    channel: Arc<dyn Channel>,
    /// The channel's own context (its network and file scope), as the router's reply uses.
    ctx: Ctx,
}

/// One post.
#[derive(Debug, Clone, Copy)]
pub struct PostRequest<'a> {
    /// A configured `[[channel]]` name.
    pub channel: &'a str,
    /// The destination (channel or thread id, topic, file recipient).
    pub to: &'a str,
    /// The message text, already bounded by the caller.
    pub text: &'a str,
    /// Idempotence key: a second post with the same `(channel, to, key)` is suppressed.
    pub key: Option<&'a str>,
    /// Per-call bound; the call returns `Timeout` when the channel is slower.
    pub timeout_ms: u64,
}

/// What a successful [`ChannelHub::post`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOutcome {
    /// Delivered now.
    Posted,
    /// The key was already used: nothing was sent.
    Duplicate,
}

/// Posts delivered and posts refused or failed, since the hub was built.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostCounts {
    /// Messages delivered.
    pub posted: u64,
    /// Calls refused or failed (capability denial, unknown channel, provider error, timeout).
    pub refused: u64,
}

#[derive(Default)]
struct KeyLog {
    seen: BTreeSet<String>,
    order: VecDeque<String>,
}

impl KeyLog {
    /// True when `k` is new (now remembered); false when already present.
    fn reserve(&mut self, k: &str) -> bool {
        if !self.seen.insert(k.to_owned()) {
            return false;
        }
        self.order.push_back(k.to_owned());
        while self.order.len() > KEY_LOG_CAP {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }

    fn release(&mut self, k: &str) {
        self.seen.remove(k);
        self.order.retain(|o| o != k);
    }
}

/// The daemon's channels by name, plus the post counters and the idempotence keys.
#[derive(Default)]
pub struct ChannelHub {
    slots: Mutex<BTreeMap<String, Slot>>,
    keys: Mutex<KeyLog>,
    posted: AtomicU64,
    refused: AtomicU64,
}

impl ChannelHub {
    /// Registers `channel` under its own name (replacing an earlier one), to be sent through `ctx`.
    pub fn insert(&self, channel: Arc<dyn Channel>, ctx: Ctx) {
        let name = channel.name().to_owned();
        self.slots.lock().insert(name, Slot { channel, ctx });
    }

    /// True when `name` is a registered channel.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.slots.lock().contains_key(name)
    }

    /// Counts a refusal that happened before the hub was reached (the capability check).
    pub fn note_refused(&self) {
        self.refused.fetch_add(1, Ordering::Relaxed);
    }

    /// The counters.
    #[must_use]
    pub fn counts(&self) -> PostCounts {
        PostCounts {
            posted: self.posted.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
        }
    }

    /// Sends `req.text` to `req.to` on `req.channel`.
    ///
    /// # Errors
    /// [`PiiError::NotFound`] for an unknown channel; [`PiiError::Invalid`] for an empty
    /// destination or text; [`PiiError::Timeout`] when the channel is slower than
    /// `req.timeout_ms` (the key, if any, stays reserved: the send may still land);
    /// otherwise the channel's own error, message preserved (bounded to [`MAX_ERROR_BYTES`]).
    pub fn post(&self, req: &PostRequest<'_>) -> Result<PostOutcome, PiiError> {
        let result = self.post_inner(req);
        match &result {
            Ok(PostOutcome::Posted) => {
                self.posted.fetch_add(1, Ordering::Relaxed);
            }
            Ok(PostOutcome::Duplicate) => {}
            Err(_) => self.note_refused(),
        }
        result
    }

    fn post_inner(&self, req: &PostRequest<'_>) -> Result<PostOutcome, PiiError> {
        let found = self
            .slots
            .lock()
            .get(req.channel)
            .map(|slot| (Arc::clone(&slot.channel), slot.ctx.clone()));
        let (channel, ctx) = found.ok_or_else(|| {
            PiiError::NotFound(format!("post: channel '{}' is not configured", req.channel))
        })?;
        if req.to.is_empty() {
            return Err(PiiError::Invalid(String::from("post: 'to' is empty")));
        }
        if req.text.is_empty() {
            return Err(PiiError::Invalid(String::from("post: text is empty")));
        }
        let key = req
            .key
            .map(|k| format!("{}\u{1f}{}\u{1f}{k}", req.channel, req.to));
        if let Some(k) = &key
            && !self.keys.lock().reserve(k)
        {
            return Ok(PostOutcome::Duplicate);
        }
        let (to, text) = (req.to.to_owned(), req.text.to_owned());
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(String::from("channel-post"))
            .spawn(move || {
                let _ = tx.send(channel.send(&ctx, &to, &text));
            });
        if let Err(e) = spawned {
            self.release(key.as_deref());
            return Err(PiiError::Internal(format!("post: cannot start send: {e}")));
        }
        match rx.recv_timeout(Duration::from_millis(req.timeout_ms)) {
            Ok(Ok(())) => Ok(PostOutcome::Posted),
            Ok(Err(e)) => {
                // A definite failure: nothing was delivered, so the retry may post.
                self.release(key.as_deref());
                Err(bound_error(e))
            }
            // The send may still land after we stopped waiting: keep the key.
            Err(_) => Err(PiiError::Timeout {
                after_ms: req.timeout_ms,
            }),
        }
    }

    fn release(&self, key: Option<&str>) {
        if let Some(k) = key {
            self.keys.lock().release(k);
        }
    }
}

/// Truncates an error's message to [`MAX_ERROR_BYTES`], keeping its variant (so retryability survives).
fn bound_error(e: PiiError) -> PiiError {
    let cut = |m: String| {
        if m.len() <= MAX_ERROR_BYTES {
            return m;
        }
        let mut end = MAX_ERROR_BYTES;
        while !m.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", m.get(..end).unwrap_or_default())
    };
    match e {
        PiiError::Invalid(m) => PiiError::Invalid(cut(m)),
        PiiError::Denied(m) => PiiError::Denied(cut(m)),
        PiiError::NotFound(m) => PiiError::NotFound(cut(m)),
        PiiError::Internal(m) => PiiError::Internal(cut(m)),
        PiiError::Unavailable(m) => PiiError::Unavailable(cut(m)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::many_single_char_names,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::print_stdout,
        clippy::disallowed_macros
    )]

    use std::time::Instant;

    use pii_core::{Budget, CapSet};

    use super::*;
    use crate::Incoming;

    /// Records deliveries; behaviour per call is scripted.
    struct Mock {
        delivered: Mutex<Vec<(String, String)>>,
        fail_first: AtomicU64,
        fail_with: PiiError,
        delay_ms: u64,
    }

    impl Mock {
        fn ok() -> Arc<Self> {
            Self::build(0, PiiError::Internal(String::new()), 0)
        }
        fn build(fail_first: u64, fail_with: PiiError, delay_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                delivered: Mutex::new(Vec::new()),
                fail_first: AtomicU64::new(fail_first),
                fail_with,
                delay_ms,
            })
        }
    }

    impl Channel for Mock {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn kind(&self) -> &'static str {
            "mock"
        }
        fn poll(&self, _: &Ctx, _: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
            Ok(Vec::new())
        }
        fn send(&self, _: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
            if self.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.delay_ms));
            }
            if self.fail_first.load(Ordering::SeqCst) > 0 {
                self.fail_first.fetch_sub(1, Ordering::SeqCst);
                return Err(self.fail_with.clone());
            }
            self.delivered.lock().push((to.to_owned(), text.to_owned()));
            Ok(())
        }
    }

    fn hub(m: &Arc<Mock>) -> ChannelHub {
        let h = ChannelHub::default();
        h.insert(
            Arc::clone(m) as Arc<dyn Channel>,
            Ctx::root(CapSet::empty(), Budget::unlimited()),
        );
        h
    }

    fn req<'a>(to: &'a str, text: &'a str, key: Option<&'a str>) -> PostRequest<'a> {
        PostRequest {
            channel: "mock",
            to,
            text,
            key,
            timeout_ms: 2000,
        }
    }

    #[test]
    fn a_post_goes_through_the_channel_send() {
        let m = Mock::ok();
        let h = hub(&m);
        assert_eq!(h.post(&req("t1", "hello", None)), Ok(PostOutcome::Posted));
        assert_eq!(*m.delivered.lock(), vec![("t1".into(), "hello".into())]);
        assert_eq!(
            h.counts(),
            PostCounts {
                posted: 1,
                refused: 0
            }
        );
    }

    #[test]
    fn an_unknown_channel_and_empty_fields_are_named_refusals() {
        let m = Mock::ok();
        let h = hub(&m);
        let mut r = req("t", "x", None);
        r.channel = "nope";
        let e = h.post(&r).unwrap_err();
        assert!(
            matches!(e, PiiError::NotFound(ref s) if s.contains("nope")),
            "{e}"
        );
        assert!(matches!(
            h.post(&req("", "x", None)),
            Err(PiiError::Invalid(_))
        ));
        assert!(matches!(
            h.post(&req("t", "", None)),
            Err(PiiError::Invalid(_))
        ));
        assert!(m.delivered.lock().is_empty());
        assert_eq!(h.counts().refused, 3);
    }

    #[test]
    fn a_provider_error_is_preserved_verbatim() {
        let body = r#"discord: send status 403: {"message": "Missing Permissions", "code": 50013}"#;
        let m = Mock::build(1, PiiError::Unavailable(body.into()), 0);
        let h = hub(&m);
        let e = h.post(&req("t", "x", None)).unwrap_err();
        assert_eq!(e, PiiError::Unavailable(body.into()));
        assert!(e.to_string().contains("50013"), "{e}");
        assert_eq!(
            h.counts(),
            PostCounts {
                posted: 0,
                refused: 1
            }
        );
    }

    #[test]
    fn an_oversized_provider_error_is_bounded_not_dropped() {
        let big = format!("start {}", "z".repeat(5000));
        let m = Mock::build(1, PiiError::Unavailable(big), 0);
        let e = hub(&m).post(&req("t", "x", None)).unwrap_err();
        let PiiError::Unavailable(s) = e else {
            panic!("{e}")
        };
        assert!(s.starts_with("start "), "{s}");
        assert!(s.len() <= MAX_ERROR_BYTES + 3, "{}", s.len());
    }

    #[test]
    fn a_retry_with_the_same_key_posts_once() {
        let m = Mock::ok();
        let h = hub(&m);
        assert_eq!(h.post(&req("t", "x", Some("k1"))), Ok(PostOutcome::Posted));
        assert_eq!(
            h.post(&req("t", "x", Some("k1"))),
            Ok(PostOutcome::Duplicate)
        );
        assert_eq!(m.delivered.lock().len(), 1);
        // another key, another destination: both post.
        assert_eq!(h.post(&req("t", "x", Some("k2"))), Ok(PostOutcome::Posted));
        assert_eq!(h.post(&req("u", "x", Some("k1"))), Ok(PostOutcome::Posted));
        assert_eq!(m.delivered.lock().len(), 3);
    }

    #[test]
    fn a_failed_send_releases_its_key_so_the_retry_posts() {
        let m = Mock::build(1, PiiError::Unavailable("boom".into()), 0);
        let h = hub(&m);
        assert!(h.post(&req("t", "x", Some("k"))).is_err());
        assert_eq!(h.post(&req("t", "x", Some("k"))), Ok(PostOutcome::Posted));
        assert_eq!(m.delivered.lock().len(), 1);
    }

    #[test]
    fn a_timeout_is_bounded_and_keeps_the_key_because_the_send_may_still_land() {
        let m = Mock::build(0, PiiError::Internal(String::new()), 1500);
        let h = hub(&m);
        let mut r = req("t", "x", Some("k"));
        r.timeout_ms = 100;
        let t0 = Instant::now();
        let e = h.post(&r).unwrap_err();
        let elapsed = t0.elapsed().as_millis();
        assert!(matches!(e, PiiError::Timeout { after_ms: 100 }), "{e}");
        assert!(
            elapsed < 1000,
            "the call must not wait for the slow channel: {elapsed} ms"
        );
        // the retry is suppressed even though the slow send eventually lands: one delivery.
        assert_eq!(h.post(&r), Ok(PostOutcome::Duplicate));
        std::thread::sleep(Duration::from_millis(1700));
        assert_eq!(m.delivered.lock().len(), 1);
    }

    #[test]
    fn the_key_log_is_bounded() {
        let mut k = KeyLog::default();
        for i in 0..(KEY_LOG_CAP + 10) {
            assert!(k.reserve(&format!("k{i}")));
        }
        assert_eq!(k.seen.len(), KEY_LOG_CAP);
        assert!(k.reserve("k0"), "the oldest key was forgotten");
    }
}
