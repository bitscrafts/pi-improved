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
