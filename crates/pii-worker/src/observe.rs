//! The observability seam of [`Supervisor`]: every lifecycle transition
//! (starting / ready / draining / restarting / failed) as a `pii-observe`
//! event.

use std::sync::Arc;

use pii_observe::{Event, Level, Observable, Observed, Observer, names};

use crate::spec::WorkerState;
use crate::supervisor::Supervisor;

impl Observable for Supervisor {
    fn observer(&self) -> Option<&dyn Observer> {
        self.inner.observer.get().map(AsRef::as_ref)
    }
}

impl Supervisor {
    /// Attaches the observer (first call wins; later calls are ignored).
    ///
    /// Set after construction rather than injected into [`Supervisor::new`]:
    /// clones share one `Inner`, and `new` has call sites (daemon, tests)
    /// that build supervisors before any observer exists.
    pub fn set_observer(&self, observer: Arc<dyn Observer>) {
        let _ = self.inner.observer.set(observer);
    }

    /// Reports that the worker entered `to`.
    pub(crate) fn note_state(&self, to: WorkerState) {
        let level = if to == WorkerState::Failed {
            Level::Error
        } else {
            Level::Info
        };
        self.emit(
            &Event::new("worker", level, names::WORKER_STATE_CHANGED)
                .field("worker", self.inner.spec.name.as_str())
                .field("state", format!("{to:?}").to_lowercase()),
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
    use crate::WorkerSpec;

    #[test]
    fn state_transition_reaches_the_observer_by_name() {
        let rec = Arc::new(Recorder::new());
        let sup = Supervisor::new(WorkerSpec {
            name: "w".into(),
            program: "/nonexistent".into(),
            args: Vec::new(),
            env: Vec::new(),
            max_in_flight: 0,
            ready_timeout_ms: 100,
            restart_backoff_ms: 10,
            restart_window_ms: 1_000,
            max_restarts_in_window: 1,
        });
        sup.set_observer(rec.clone());
        // No SPAWN capability: start fails after announcing the spawn attempt.
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        assert!(sup.start(&ctx).is_err());
        let ev = rec.events_named(names::WORKER_STATE_CHANGED);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].fields.get("worker"), Some(&"w".into()));
        assert_eq!(ev[0].fields.get("state"), Some(&"starting".into()));
    }
}
