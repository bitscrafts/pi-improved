//! The engine's observer, audit, trajectory and daemon-context attachments and `cancel_all` (split from `lib.rs` for the
//! 450-line gate). A second `impl Engine`: every caller is unchanged.

use std::path::PathBuf;
use std::sync::Arc;

use pii_abi::RunId;

use crate::{Engine, daemon};

impl Engine {
    /// Attaches the shared engine handle the `spawn` node uses to launch
    /// child runs (spec 32 R1). Called once by the daemon (or a test) after
    /// `Engine::new` — the registry is built inside `new`, so the factory
    /// cannot hold the `Arc<Engine>` directly.
    pub fn attach_spawn_engine(&self, engine: Arc<Self>) {
        *self.spawn_handle.lock() = Some(engine);
    }

    /// Attaches the daemon's runtime context (F5.1), set once at daemon
    /// startup after the engine is already `Arc`-shared.
    pub fn attach_daemon_ctx(&self, ctx: Arc<daemon::status::DaemonContext>) {
        *self.daemon_ctx.lock() = Some(ctx);
    }

    /// Attaches an observer for events raised outside a run's own `Ctx`
    /// (D3/D6, 026 report 5.0.4) — e.g. so `session.persist` failures reach
    /// the same observer a daemon wires up for everything else. Optional:
    /// a bare `Engine` (tests, oneshot) that never calls this gets
    /// [`pii_core::NullObserver`] semantics from [`Engine::observer`].
    pub fn attach_observer(&self, obs: Arc<dyn pii_core::Observer>) {
        *self.obs.lock() = Some(obs);
    }

    /// Attaches the trajectory sink (spec 103): every run spawned from now on
    /// writes its journal and verbatim outcome to it. Default is no sink.
    pub fn attach_trajectory(&self, sink: Arc<pii_observe::TrajectorySink>) {
        *self.trajectory.lock() = Some(sink);
    }

    /// Records the trajectory root the promotion lifecycle reads to measure a
    /// promotion's outcome (spec 125). Without one, windows measure zero runs.
    pub fn attach_learn_traj_root(&self, root: PathBuf) {
        *self.learn_traj_root.lock() = Some(root);
    }

    /// The root recorded by [`Engine::attach_learn_traj_root`].
    #[must_use]
    pub fn learn_traj_root(&self) -> Option<PathBuf> {
        self.learn_traj_root.lock().clone()
    }

    /// Attaches the contract observer (the daemon log) that the store, the
    /// model provider, the network service, workers and channels report to
    /// (spec 108). Until called, those subsystems' events are dropped.
    pub fn attach_log(&self, obs: Arc<dyn pii_observe::Observer>) {
        self.log.attach(obs);
    }

    /// The late-bound contract observer, for subsystems built after the
    /// engine (the daemon's workers, channels and network service).
    #[must_use]
    pub fn log(&self) -> Arc<dyn pii_observe::Observer> {
        self.log.clone()
    }

    /// Attaches the audit observer (spec 107 R3): every run's denial events
    /// (capability, tool deny-list, scope) reach it, in addition to the run's
    /// own bus and trajectory observers. Default is none.
    pub fn attach_audit(&self, obs: Arc<dyn pii_core::Observer>) {
        *self.audit.lock() = Some(obs);
    }

    /// Detaches the observer fan-out from every run started from now on
    /// (spec 116, **measurement only**; the default is attached): no
    /// tool-call/log observer is added to a run's context. The bus bridge, the
    /// audit feed and the trajectory sink are separate surfaces and unaffected.
    pub fn detach_observers(&self) {
        self.observers_detached
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether [`Engine::detach_observers`] was called.
    #[must_use]
    pub fn observers_detached(&self) -> bool {
        self.observers_detached
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The audit observer, if one is attached.
    pub(crate) fn audit_observer(&self) -> Option<Arc<dyn pii_core::Observer>> {
        self.audit.lock().clone()
    }

    /// The per-run observer feeding the trajectory sink, if one is attached.
    pub(crate) fn trajectory_observer(
        &self,
        run: &RunId,
        session: Option<&str>,
    ) -> Option<Arc<dyn pii_core::Observer>> {
        let sink = self.trajectory.lock().clone()?;
        Some(Arc::new(pii_core::TrajectoryBridge::new(
            sink, run, session,
        )))
    }

    /// The engine's out-of-band observer (D3/D6): the one attached via
    /// [`Engine::attach_observer`], or a fresh [`pii_core::NullObserver`]
    /// when none was ever attached.
    pub(crate) fn observer(&self) -> Arc<dyn pii_core::Observer> {
        self.obs
            .lock()
            .clone()
            .unwrap_or_else(|| Arc::new(pii_core::NullObserver))
    }

    /// Reports a durable-side failure that cannot be propagated to a caller
    /// (spec 106 R4) through the engine's out-of-band observer.
    pub(crate) fn report_unhandled(&self, context: &str, message: &str) {
        self.observer().event(&pii_core::ObsEvent::PersistError {
            context: context.to_owned(),
            message: message.to_owned(),
        });
    }

    /// The daemon's runtime context, when this engine is running under
    /// `pii daemon` (F5.1). `None` for a bare `Engine`.
    #[must_use]
    pub fn daemon_ctx(&self) -> Option<Arc<daemon::status::DaemonContext>> {
        self.daemon_ctx.lock().clone()
    }

    /// Trips every active run's cancel token (F5.3 graceful shutdown).
    pub fn cancel_all(&self) {
        let ids: Vec<String> = self.runs.lock().keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }
}
