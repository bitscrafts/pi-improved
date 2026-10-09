//! The observer trait and its no-op helpers.

use crate::{Event, Measure};

/// Receives events and measures. All methods default to no-ops;
/// [`NullObserver`] is the canonical no-op.
pub trait Observer: Send + Sync {
    /// Called with each event.
    fn event(&self, _e: &Event) {}
    /// Called with each measure, tagged by name.
    fn measure(&self, _m: &Measure) {}
    /// Flushes any buffered output.
    fn flush(&self) {}
}

/// The canonical no-op observer.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullObserver;

impl Observer for NullObserver {}

/// Alias-style helper kept for ergonomic naming at call sites.
pub type NoopObserver = NullObserver;

/// An observer that records everything it is told, for tests that assert on
/// the names a subsystem emitted.
#[derive(Debug, Default)]
// reason: pii-observe is the dependency floor (serde only); it cannot take
// parking_lot, and a poisoned lock here just drops a recording.
#[allow(clippy::disallowed_types)]
pub struct Recorder {
    events: std::sync::Mutex<Vec<crate::OwnedEvent>>,
    measures: std::sync::Mutex<Vec<Measure>>,
}

impl Recorder {
    /// A fresh recorder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event received so far, in order.
    #[must_use]
    pub fn events(&self) -> Vec<crate::OwnedEvent> {
        self.events.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Every measure received so far, in order.
    #[must_use]
    pub fn measures(&self) -> Vec<Measure> {
        self.measures.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// The events whose kind is `kind`.
    #[must_use]
    pub fn events_named(&self, kind: &str) -> Vec<crate::OwnedEvent> {
        self.events()
            .into_iter()
            .filter(|e| e.kind == kind)
            .collect()
    }
}

impl Observer for Recorder {
    fn event(&self, e: &Event) {
        if let Ok(mut g) = self.events.lock() {
            g.push(crate::OwnedEvent::from(e));
        }
    }
    fn measure(&self, m: &Measure) {
        if let Ok(mut g) = self.measures.lock() {
            g.push(*m);
        }
    }
}

/// The per-crate seam: a component that may have an observer attached.
pub trait Observable {
    /// The attached observer, if any.
    fn observer(&self) -> Option<&dyn Observer>;
}

/// Ergonomic no-op-safe emission for any [`Observable`].
pub trait Observed: Observable {
    /// Emits an event if an observer is attached.
    fn emit(&self, e: &Event) {
        if let Some(o) = self.observer() {
            o.event(e);
        }
    }

    /// Records a measure if an observer is attached.
    fn record(&self, m: &Measure) {
        if let Some(o) = self.observer() {
            o.measure(m);
        }
    }
}

impl<T: Observable + ?Sized> Observed for T {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{Level, names};

    #[derive(Default)]
    struct Count(AtomicUsize, AtomicUsize);

    impl Observer for Count {
        fn event(&self, _e: &Event) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn measure(&self, _m: &Measure) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Comp(Option<Count>);

    impl Observable for Comp {
        fn observer(&self) -> Option<&dyn Observer> {
            self.0.as_ref().map(|c| c as &dyn Observer)
        }
    }

    #[test]
    fn observed_forwards_when_attached_and_is_silent_when_not() {
        let e = Event::new("graph", Level::Info, names::GRAPH_NODE_DEAD);
        let m = Measure::Counter(1);
        let with = Comp(Some(Count::default()));
        with.emit(&e);
        with.record(&m);
        let c = with.0.as_ref().unwrap();
        assert_eq!(
            (c.0.load(Ordering::SeqCst), c.1.load(Ordering::SeqCst)),
            (1, 1)
        );
        let without = Comp(None);
        without.emit(&e);
        without.record(&m);
        NullObserver.event(&e);
        NullObserver.flush();
    }
}
