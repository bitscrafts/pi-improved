//! Test seams (spec 120 R1): hooks that let a test force one exact
//! interleaving of the supervisor's threads instead of hoping a loaded
//! machine produces it. Compiled out of non-test builds: the call sites are
//! no-ops there.

use crate::supervisor::Supervisor;

#[cfg(test)]
use {parking_lot::Mutex, std::sync::Arc};

#[cfg(test)]
type Hook = Arc<dyn Fn() + Send + Sync>;
#[cfg(test)]
type EofHook = Arc<dyn Fn(bool) + Send + Sync>;

/// The installed hooks (all empty by default).
#[cfg(test)]
#[derive(Default)]
#[allow(unreachable_pub)] // reason: clippy rejects pub(crate) in a private module and pub as unreachable
pub struct Seams {
    /// Runs inside `invoke`, right after the state decision.
    pub invoke_gap: Mutex<Option<Hook>>,
    /// Runs inside `reload`, after the old child is gone and before respawn.
    pub reload_gap: Mutex<Option<Hook>>,
    /// Runs around a reader thread's `death` call: `false` before, `true` after.
    pub reader_eof: Mutex<Option<EofHook>>,
}

impl Supervisor {
    #[cfg(test)]
    pub(crate) fn seam_invoke_gap(&self) {
        let h = self.inner.seams.invoke_gap.lock().clone();
        if let Some(h) = h {
            h();
        }
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)] // reason: same signature as the test build's hook caller
    pub(crate) fn seam_invoke_gap(&self) {}

    #[cfg(test)]
    pub(crate) fn seam_reload_gap(&self) {
        let h = self.inner.seams.reload_gap.lock().clone();
        if let Some(h) = h {
            h();
        }
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)] // reason: same signature as the test build's hook caller
    pub(crate) fn seam_reload_gap(&self) {}

    #[cfg(test)]
    pub(crate) fn seam_reader_eof(&self, after: bool) {
        let h = self.inner.seams.reader_eof.lock().clone();
        if let Some(h) = h {
            h(after);
        }
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)] // reason: same signature as the test build's hook caller
    pub(crate) fn seam_reader_eof(&self, _after: bool) {}
}
