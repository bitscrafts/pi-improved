//! The observability seam of [`NetService`]: egress denials, dial failures
//! and accept failures as `pii-observe` events.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_observe::{Event, Level, Observable, Observed, Observer, names};

use super::NetService;
use crate::authorize::Target;

impl Observable for NetService {
    fn observer(&self) -> Option<&dyn Observer> {
        self.0.observer.get().map(AsRef::as_ref)
    }
}

impl NetService {
    /// Attaches the observer (first call wins; later calls are ignored).
    ///
    /// Set after construction rather than injected into [`crate::NetConfig`]:
    /// clones of the service share one `Inner`, and `NetConfig` literals are
    /// built at many call sites that have no observer.
    pub fn set_observer(&self, observer: Arc<dyn Observer>) {
        let _ = self.0.observer.set(observer);
    }

    /// Reports why a `connect` failed: a denial is an egress denial, anything
    /// else a dial failure. Carries host/port and grant key, never payloads.
    pub(super) fn observe_connect_failure(&self, target: &Target, key: &str, e: &PiiError) {
        let (level, kind) = if matches!(e, PiiError::Denied(_)) {
            (Level::Warn, names::NET_EGRESS_DENIED)
        } else {
            (Level::Error, names::NET_DIAL_FAILED)
        };
        self.emit(
            &Event::new("net", level, kind)
                .field("host", target.host.as_str())
                .field("port", u64::from(target.port))
                .field("key", key)
                .field("error", e.to_string()),
        );
    }

    /// Reports a listener `accept` failure that ended the accept loop.
    pub(crate) fn observe_accept_failure(&self, e: &std::io::Error) {
        self.emit(
            &Event::new("net", Level::Error, names::NET_ACCEPT_FAILED)
                .field("error", e.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_core::{Budget, CapSet, Ctx};
    use pii_observe::Recorder;

    use super::*;
    use crate::{NetConfig, Scheme};

    fn svc() -> NetService {
        NetService::new(NetConfig {
            resolver: Arc::new(crate::resolve::StaticResolver::new()),
            grants: Arc::new(crate::grant::StaticGrants::default()),
            clock: Arc::new(crate::clock::SystemClock),
        })
    }

    #[test]
    fn egress_denial_reaches_the_observer_by_name() {
        let rec = Arc::new(Recorder::new());
        let svc = svc();
        svc.set_observer(rec.clone());
        let target = Target {
            host: "example.invalid".into(),
            port: 80,
        };
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let err = svc.connect(&ctx, &target, "http:example.invalid", 100, Scheme::Http);
        assert!(matches!(err, Err(PiiError::Denied(_))));
        let ev = rec.events_named(names::NET_EGRESS_DENIED);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].fields.get("host"), Some(&"example.invalid".into()));
    }

    #[test]
    fn accept_failure_reaches_the_observer_by_name() {
        let rec = Arc::new(Recorder::new());
        let svc = svc();
        svc.set_observer(rec.clone());
        svc.observe_accept_failure(&std::io::Error::other("boom"));
        assert_eq!(rec.events_named(names::NET_ACCEPT_FAILED).len(), 1);
    }
}
