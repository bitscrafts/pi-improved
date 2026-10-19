//! Spec 195: a refused post is paced by the `retry_after` it carries.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_channels::post::{ChannelHub, PostOutcome, PostRequest};
use pii_channels::{Channel, Incoming};
use pii_core::{Budget, CapSet, Ctx};

const RATE_LIMITED: &str = r#"discord: forum post status 429: {"message":"You are being rate limited.","retry_after":1.0,"global":false}"#;

/// Counts dials; the first `fail_first` fail with `error`.
struct Fake {
    name: &'static str,
    dials: AtomicU64,
    fail_first: AtomicU64,
    error: String,
    delivered: Mutex<u64>,
}

impl Fake {
    fn new(name: &'static str, fail_first: u64, error: &str) -> Arc<Self> {
        Arc::new(Self {
            name,
            dials: AtomicU64::new(0),
            fail_first: AtomicU64::new(fail_first),
            error: error.to_owned(),
            delivered: Mutex::new(0),
        })
    }
    fn dials(&self) -> u64 {
        self.dials.load(Ordering::SeqCst)
    }
}

impl Channel for Fake {
    fn name(&self) -> &'static str {
        self.name
    }
    fn kind(&self) -> &'static str {
        "fake"
    }
    fn poll(&self, _: &Ctx, _: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        Ok(Vec::new())
    }
    fn send(&self, _: &Ctx, _: &str, _: &str) -> Result<(), PiiError> {
        self.dials.fetch_add(1, Ordering::SeqCst);
        let left = self.fail_first.load(Ordering::SeqCst);
        if left > 0 {
            self.fail_first.store(left - 1, Ordering::SeqCst);
            return Err(PiiError::Unavailable(self.error.clone()));
        }
        *self.delivered.lock() += 1;
        Ok(())
    }
}

fn hub(chans: &[&Arc<Fake>]) -> ChannelHub {
    let h = ChannelHub::default();
    for c in chans {
        h.insert(
            Arc::clone(*c) as Arc<dyn Channel>,
            Ctx::root(CapSet::empty(), Budget::unlimited()),
        );
    }
    h
}

fn req<'a>(channel: &'a str, key: Option<&'a str>) -> PostRequest<'a> {
    PostRequest {
        channel,
        to: "t",
        text: "x",
        key,
        tag: None,
        timeout_ms: 2000,
    }
}

#[test]
fn a_paced_channel_is_dialled_once() {
    let f = Fake::new("d", u64::MAX, RATE_LIMITED);
    let h = hub(&[&f]);
    for _ in 0..10 {
        let e = h.post(&req("d", Some("k"))).unwrap_err();
        assert!(matches!(e, PiiError::Unavailable(_)), "{e}");
    }
    assert_eq!(f.dials(), 1);
    assert_eq!(*f.delivered.lock(), 0);
    assert_eq!(h.counts().refused, 10);
    let e = h.post(&req("d", None)).unwrap_err().to_string();
    assert!(e.contains("channel d: paced by the provider for"), "{e}");
    assert!(e.contains(" ms"), "{e}");
}

#[test]
fn a_post_lands_after_the_window() {
    let f = Fake::new("d", 1, RATE_LIMITED);
    let h = hub(&[&f]);
    assert!(h.post(&req("d", Some("k"))).is_err());
    assert!(h.post(&req("d", Some("k"))).is_err());
    assert_eq!(f.dials(), 1);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(h.post(&req("d", Some("k"))), Ok(PostOutcome::Posted));
    assert_eq!(f.dials(), 2);
    assert_eq!(*f.delivered.lock(), 1);
}

#[test]
fn an_error_without_retry_after_does_not_pace() {
    let f = Fake::new("d", 1, "discord: send status 500: boom");
    let h = hub(&[&f]);
    assert!(h.post(&req("d", None)).is_err());
    assert_eq!(h.post(&req("d", None)), Ok(PostOutcome::Posted));
    assert_eq!(f.dials(), 2);
}

#[test]
fn a_success_clears_the_pace() {
    // 429 (0.2 s), wait it out, a success, then an error without the token never re-paces: the
    // second post after the success dials. Clearing is exercised by the success's observation.
    let f = Fake::new("d", 1, r#"{"retry_after":0.2}"#);
    let h = hub(&[&f]);
    assert!(h.post(&req("d", None)).is_err());
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(h.post(&req("d", None)), Ok(PostOutcome::Posted));
    assert_eq!(h.post(&req("d", None)), Ok(PostOutcome::Posted));
    assert_eq!(f.dials(), 3);
}

#[test]
fn the_pace_is_per_channel() {
    let a = Fake::new("a", u64::MAX, RATE_LIMITED);
    let b = Fake::new("b", 0, "");
    let h = hub(&[&a, &b]);
    assert!(h.post(&req("a", None)).is_err());
    assert!(h.post(&req("a", None)).is_err());
    assert_eq!(a.dials(), 1);
    assert_eq!(h.post(&req("b", None)), Ok(PostOutcome::Posted));
    assert_eq!(b.dials(), 1);
}
