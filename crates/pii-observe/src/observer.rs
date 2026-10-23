//! The observer trait and its no-op helpers.

use crate::{Event, Level, Measure};

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

/// An observer whose target is attached after construction (spec 108).
///
/// Subsystems that are built before the daemon's log exists (the engine opens
/// its store, provider and network service in `Engine::new`) are handed one
/// of these; until [`SwitchObserver::attach`] is called it drops everything,
/// afterwards it forwards to the attached observer.
#[derive(Default)]
pub struct SwitchObserver {
    target: std::sync::RwLock<Option<std::sync::Arc<dyn Observer>>>,
    /// Events and measures that arrived while no target was attached.
    dropped: std::sync::atomic::AtomicU64,
}

impl SwitchObserver {
    /// An unattached switch (drops everything).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches `target`; later calls replace it. When anything was dropped
    /// while unattached, the new target is told how much
    /// ([`crate::names::OBSERVE_EVENTS_DROPPED`], field `count`; spec 109 R2).
    pub fn attach(&self, target: std::sync::Arc<dyn Observer>) {
        if let Ok(mut g) = self.target.write() {
            *g = Some(target);
        }
        let count = self.dropped.swap(0, std::sync::atomic::Ordering::Relaxed);
        if count > 0 {
            self.event(
                &Event::new("observe", Level::Warn, crate::names::OBSERVE_EVENTS_DROPPED)
                    .field("count", count),
            );
        }
    }

    fn with<R>(&self, f: impl FnOnce(&dyn Observer) -> R) -> Option<R> {
        let g = self.target.read().ok()?;
        g.as_deref().map(f)
    }

    fn forward_or_count(&self, f: impl FnOnce(&dyn Observer)) {
        if self.with(f).is_none() {
            self.dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl Observer for SwitchObserver {
    fn event(&self, e: &Event) {
        self.forward_or_count(|o| o.event(e));
    }
    fn measure(&self, m: &Measure) {
        self.forward_or_count(|o| o.measure(m));
    }
    fn flush(&self) {
        self.with(|o| o.flush());
    }
}

/// Alias-style helper kept for ergonomic naming at call sites.
pub type NoopObserver = NullObserver;

/// An observer for command-line binaries: writes each event to stderr as
/// `<subsystem>: <kind> <fields-json>`. Measures are ignored.
#[derive(Debug, Clone, Copy, Default)]
pub struct StderrObserver;

impl StderrObserver {
    /// The line an event is written as.
    #[must_use]
    pub fn render(e: &Event) -> String {
        let fields = serde_json::to_string(&e.fields).unwrap_or_default();
        format!("{}: {} {fields}", e.subsystem, e.kind)
    }
}

impl Observer for StderrObserver {
    fn event(&self, e: &Event) {
        use std::io::Write as _;
        // reason: nowhere to report a failed stderr write; dropping is correct.
        let _ = writeln!(std::io::stderr(), "{}", Self::render(e));
    }
}

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
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

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
    fn switch_drops_until_attached_then_forwards() {
        let sw = SwitchObserver::new();
        let rec = std::sync::Arc::new(Recorder::new());
        let e = Event::new("graph", Level::Info, names::GRAPH_NODE_DEAD);
        sw.event(&e);
        sw.attach(rec.clone());
        sw.event(&e);
        sw.measure(&Measure::Counter(1));
        assert_eq!(
            rec.events_named(names::GRAPH_NODE_DEAD).len(),
            1,
            "only the post-attach event arrives"
        );
        assert_eq!(rec.events_named(names::OBSERVE_EVENTS_DROPPED).len(), 1);
        assert_eq!(rec.measures().len(), 1);
    }

    #[test]
    fn stderr_observer_renders_subsystem_kind_and_fields() {
        let e = Event::new("cli", Level::Error, names::CLI_FAILURE).field("why", "x");
        assert_eq!(
            StderrObserver::render(&e),
            r#"cli: cli.failure {"why":"x"}"#
        );
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

    #[test]
    fn events_dropped_before_attach_are_counted_and_reported_once() {
        let sw = SwitchObserver::new();
        sw.event(&Event::new("t", Level::Info, names::TOOLS_CALL));
        sw.event(&Event::new("t", Level::Info, names::TOOLS_CALL));
        sw.measure(&Measure::Counter(1));
        let rec = std::sync::Arc::new(Recorder::new());
        sw.attach(rec.clone());
        let ev = rec.events_named(names::OBSERVE_EVENTS_DROPPED);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].fields.get("count"), Some(&serde_json::json!(3)));
        // Nothing dropped since: a re-attach reports nothing more.
        let again = std::sync::Arc::new(Recorder::new());
        sw.attach(again.clone());
        assert_eq!(again.events_named(names::OBSERVE_EVENTS_DROPPED).len(), 0);
    }

    #[test]
    fn attaching_with_nothing_dropped_reports_nothing() {
        let sw = SwitchObserver::new();
        let rec = std::sync::Arc::new(Recorder::new());
        sw.attach(rec.clone());
        sw.event(&Event::new("t", Level::Info, names::TOOLS_CALL));
        assert_eq!(rec.events_named(names::OBSERVE_EVENTS_DROPPED).len(), 0);
        assert_eq!(rec.events_named(names::TOOLS_CALL).len(), 1);
    }
}
