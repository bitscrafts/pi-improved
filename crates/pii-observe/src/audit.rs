//! The audit trail: the security-relevant subset of events — denials and the
//! sandbox profile in force — routed to a file of their own, so an operator can
//! read them after the fact without wading through (or enabling) the full log.
//!
//! A denial raised INSIDE a child process by the kernel (a raw
//! `echo > /outside` blocked by Landlock) never passes through pii and is not
//! here; only denials pii itself decides appear.

use std::sync::Arc;

use crate::{Event, Observer, names};

/// The event kinds that belong in the audit trail.
pub const AUDIT_KINDS: &[&str] = &[
    names::CORE_CAP_DENIED,
    names::TOOLS_DENY_LIST_HIT,
    names::FETCH_ALLOWLIST_DENIED,
    names::NET_EGRESS_DENIED,
    names::DAEMON_SANDBOX_IN_FORCE,
    names::DAEMON_PEER_REFUSED,
    names::DAEMON_OBSERVER_DETACHED,
    names::DAEMON_AUDIT_DISABLED,
];

/// The only kind the trail still records once it is switched off (spec 116):
/// the fact that it was switched off.
pub const AUDIT_OFF_KINDS: &[&str] = &[names::DAEMON_AUDIT_DISABLED];

/// Forwards only [`AUDIT_KINDS`] events (or the given kinds) to `inner`.
pub struct AuditFilter {
    inner: Arc<dyn Observer>,
    kinds: &'static [&'static str],
}

impl AuditFilter {
    /// A filter in front of `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn Observer>) -> Self {
        Self::with_kinds(inner, AUDIT_KINDS)
    }

    /// A filter that lets only `kinds` through (the audit-off trail, spec 116).
    #[must_use]
    pub fn with_kinds(inner: Arc<dyn Observer>, kinds: &'static [&'static str]) -> Self {
        Self { inner, kinds }
    }
}

impl Observer for AuditFilter {
    fn event(&self, e: &Event) {
        if self.kinds.contains(&e.kind) {
            self.inner.event(e);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Sends every event to each of several observers.
pub struct Fanout(Vec<Arc<dyn Observer>>);

impl Fanout {
    /// A fan-out over `observers`.
    #[must_use]
    pub fn new(observers: Vec<Arc<dyn Observer>>) -> Self {
        Self(observers)
    }
}

impl Observer for Fanout {
    fn event(&self, e: &Event) {
        for o in &self.0 {
            o.event(e);
        }
    }

    fn measure(&self, m: &crate::Measure) {
        for o in &self.0 {
            o.measure(m);
        }
    }

    fn flush(&self) {
        for o in &self.0 {
            o.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::{Level, Recorder};

    #[test]
    fn only_audit_kinds_pass_the_filter() {
        let rec = Arc::new(Recorder::new());
        let f = AuditFilter::new(rec.clone());
        f.event(&Event::new("core", Level::Warn, names::CORE_CAP_DENIED).field("cap", "fs_write"));
        f.event(&Event::new("tools", Level::Info, names::TOOLS_CALL));
        f.event(&Event::new(
            "tools",
            Level::Warn,
            names::TOOLS_DENY_LIST_HIT,
        ));
        let kinds: Vec<_> = rec.events().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [names::CORE_CAP_DENIED, names::TOOLS_DENY_LIST_HIT]);
    }

    #[test]
    fn the_audit_off_filter_records_only_that_it_is_off() {
        let rec = Arc::new(Recorder::new());
        let f = AuditFilter::with_kinds(rec.clone(), AUDIT_OFF_KINDS);
        f.event(&Event::new("core", Level::Warn, names::CORE_CAP_DENIED));
        f.event(&Event::new(
            "daemon",
            Level::Warn,
            names::DAEMON_AUDIT_DISABLED,
        ));
        let kinds: Vec<_> = rec.events().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [names::DAEMON_AUDIT_DISABLED]);
    }

    #[test]
    fn every_audit_kind_is_a_registered_name() {
        for k in AUDIT_KINDS {
            assert!(names::ALL.contains(k), "{k} not registered");
        }
    }

    #[test]
    fn fanout_reaches_every_observer() {
        let (a, b) = (Arc::new(Recorder::new()), Arc::new(Recorder::new()));
        let f = Fanout::new(vec![a.clone(), b.clone()]);
        f.event(&Event::new("daemon", Level::Info, names::DAEMON_STARTED));
        assert_eq!((a.events().len(), b.events().len()), (1, 1));
    }
}
