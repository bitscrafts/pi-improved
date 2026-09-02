//! Observability: events, metrics, and observer multiplexing.

use std::sync::Arc;

use parking_lot::Mutex;

/// An observation about the engine's execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsEvent {
    /// A journal event was appended.
    Run(pii_abi::RunEvent),
    /// A capability denial occurred.
    Denial {
        /// The principal that was denied.
        principal: String,
        /// The capability that was denied.
        cap: String,
    },
}

/// A named numeric metric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metric {
    /// Metric name.
    pub name: String,
    /// Metric value.
    pub value: u64,
}

/// Subscriber to engine events and metrics.
pub trait Observer: Send + Sync + 'static {
    /// Called with each observation.
    fn event(&self, e: &ObsEvent);
    /// Called with each metric update.
    fn metric(&self, m: &Metric);
    /// Flushes any buffered output.
    fn flush(&self);
}

/// An observer that ignores everything.
pub struct NullObserver;

impl Observer for NullObserver {
    fn event(&self, _e: &ObsEvent) {}
    fn metric(&self, _m: &Metric) {}
    fn flush(&self) {}
}

/// An in-memory observer that records everything it is told.
pub struct RecordingObserver {
    metrics: Mutex<Vec<Metric>>,
    events: Mutex<Vec<ObsEvent>>,
}

impl RecordingObserver {
    /// A fresh observer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metrics: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    /// All recorded metrics.
    #[must_use]
    pub fn metrics(&self) -> Vec<Metric> {
        self.metrics.lock().clone()
    }

    /// The maximum value recorded for a metric name (0 if never set).
    #[must_use]
    pub fn max_metric(&self, name: &str) -> u64 {
        self.metrics
            .lock()
            .iter()
            .filter(|m| m.name == name)
            .map(|m| m.value)
            .max()
            .unwrap_or(0)
    }

    /// All recorded events.
    #[must_use]
    pub fn events(&self) -> Vec<ObsEvent> {
        self.events.lock().clone()
    }
}

impl Default for RecordingObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer for RecordingObserver {
    fn event(&self, e: &ObsEvent) {
        self.events.lock().push(e.clone());
    }

    fn metric(&self, m: &Metric) {
        self.metrics.lock().push(m.clone());
    }

    fn flush(&self) {}
}

/// Observes by fanning out to several observers.
pub struct MultiObserver(Vec<Arc<dyn Observer>>);

impl MultiObserver {
    /// A multiplexer over the given observers.
    #[must_use]
    pub fn new(observers: Vec<Arc<dyn Observer>>) -> Self {
        Self(observers)
    }
}

impl Observer for MultiObserver {
    fn event(&self, e: &ObsEvent) {
        for o in &self.0 {
            o.event(e);
        }
    }

    fn metric(&self, m: &Metric) {
        for o in &self.0 {
            o.metric(m);
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

    use std::sync::Arc;

    use super::{Metric, Observer, RecordingObserver};

    #[test]
    fn recording_observer_reports_max_metric() {
        let o = Arc::new(RecordingObserver::new());
        for v in [1u64, 2, 1] {
            o.metric(&Metric {
                name: String::from("graph.batch.width"),
                value: v,
            });
        }
        assert_eq!(o.max_metric("graph.batch.width"), 2);
        assert_eq!(o.max_metric("missing"), 0);
    }
}
