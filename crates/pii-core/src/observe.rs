//! Observability: events, metrics, and observer multiplexing.

use std::sync::Arc;

use parking_lot::Mutex;

/// An observation about the engine's execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsEvent {
    /// A journal event was appended (unsequenced form, for callers that
    /// emit events outside a run journal).
    Run(pii_abi::RunEvent),
    /// A journal event was appended at the given sequence number. Emitted by
    /// the run engine so observers can stream `{"seq":n,"event":..}` lines.
    Journal {
        /// The journal sequence the event was written at.
        seq: u64,
        /// The appended event.
        event: pii_abi::RunEvent,
    },
    /// A capability denial occurred.
    Denial {
        /// The principal that was denied.
        principal: String,
        /// The capability that was denied.
        cap: String,
    },
    /// A durable-store write failed outside the run journal (D3/D6, 026
    /// report 5.0.4) — e.g. a session append/snapshot. The Phase-1
    /// journaler has its own `Result` return for callers that check it
    /// directly; this variant exists for call sites (like session
    /// persistence) that are best-effort by design and must not panic or
    /// abort on a write failure, but also must not silently swallow it.
    PersistError {
        /// What was being persisted (e.g. `"session:<id>"`).
        context: String,
        /// The error text.
        message: String,
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

/// Bridges the legacy [`Observer`] surface onto the `pii-observe` contract.
///
/// Metrics and persist/denial events a subsystem raises through a [`crate::Ctx`]
/// reach the contract observer as named [`pii_observe::Event`]s. Metric names
/// that are not registered in [`pii_observe::names`] are dropped (an event's
/// kind must be a registered `&'static str`).
pub struct ContractObserver(Arc<dyn pii_observe::Observer>);

impl ContractObserver {
    /// Forwards to `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn pii_observe::Observer>) -> Self {
        Self(inner)
    }
}

impl Observer for ContractObserver {
    fn event(&self, e: &ObsEvent) {
        use pii_observe::{Event, Level, names};
        match e {
            ObsEvent::Denial { principal, cap } => self.0.event(
                &Event::new("core", Level::Warn, names::CORE_CAP_DENIED)
                    .field("principal", principal.as_str())
                    .field("cap", cap.as_str()),
            ),
            ObsEvent::PersistError { context, message } => self.0.event(
                &Event::new("store", Level::Error, names::STORE_PERSIST_ERROR)
                    .field("context", context.as_str())
                    .field("message", message.as_str()),
            ),
            ObsEvent::Run(_) | ObsEvent::Journal { .. } => {}
        }
    }

    fn metric(&self, m: &Metric) {
        use pii_observe::{Event, Level, names};
        if let Some(kind) = names::ALL.iter().find(|n| **n == m.name) {
            self.0
                .event(&Event::new("graph", Level::Debug, kind).field("value", m.value));
        }
    }

    fn flush(&self) {
        self.0.flush();
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

    use std::sync::Arc;

    use super::{ContractObserver, Metric, Observer, RecordingObserver};

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

    #[test]
    fn contract_observer_maps_registered_metrics_and_drops_unknown() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let o = ContractObserver::new(rec.clone());
        o.metric(&Metric {
            name: String::from(pii_observe::names::GRAPH_NODE_DEAD),
            value: 2,
        });
        o.metric(&Metric {
            name: String::from("not.registered"),
            value: 1,
        });
        let evs = rec.events();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, pii_observe::names::GRAPH_NODE_DEAD);
        assert_eq!(evs[0].fields.get("value"), Some(&serde_json::json!(2)));
    }
}
