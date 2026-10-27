//! Spec 289: a running node's agent-call observations reach the journal while the node still runs.
//!
//! The node's observer records each observation (see [`CallRecorder`]) and wakes the barrier's watcher. The watcher
//! journals what the node has recorded, in the order it was recorded, and flushes it within the window, so a daemon that
//! dies mid-call leaves the `coder.spawned` in the store. What the watcher has not taken is drained after the node
//! returns, so each observation is journalled exactly once.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};

use pii_abi::{NodeId, PiiError, RunEvent};
use pii_core::{Metric, ObsEvent, Observer};

use super::barrier::START_DURABLE_WITHIN;
use super::dispatch::Env;
use crate::agent_calls::CallRecorder;

/// What the watcher hears from a running node.
pub(super) enum Tick {
    /// The node recorded an observation.
    Recorded,
    /// The node returned; the watch ends.
    Done,
}

/// One node run's recorder, and the channel that wakes its watcher.
pub(super) struct Calls {
    recorder: Arc<CallRecorder>,
    tx: Sender<Tick>,
}

impl Calls {
    /// A recorder for `node` that forwards to `inner`, and the receiving end of its wake channel.
    pub(super) fn start(node: NodeId, inner: Arc<dyn Observer>) -> (Self, Receiver<Tick>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let calls = Self {
            recorder: Arc::new(CallRecorder::new(node, inner)),
            tx,
        };
        (calls, rx)
    }

    /// The observer the node's context carries: records each observation and wakes the watcher.
    pub(super) fn observer(&self) -> Arc<dyn Observer> {
        Arc::new(Wake {
            recorder: Arc::clone(&self.recorder),
            tx: self.tx.clone(),
        })
    }

    /// Ends the watch when dropped, so the watcher stops even if the node's run unwinds.
    pub(super) fn finish(&self) -> Finish {
        Finish(self.tx.clone())
    }

    /// The observations not yet journalled, in the order the node made them.
    pub(super) fn take(&self) -> Vec<RunEvent> {
        self.recorder.take()
    }
}

/// Forwards every observation to the recorder, and wakes the watcher when the node records one.
struct Wake {
    recorder: Arc<CallRecorder>,
    tx: Sender<Tick>,
}

impl Observer for Wake {
    fn event(&self, e: &ObsEvent) {
        self.recorder.event(e);
        if let ObsEvent::NodeObserved { .. } = e {
            // The watcher may already have ended; a missed wake only defers the journalling to the post-run drain.
            drop(self.tx.send(Tick::Recorded));
        }
    }

    fn metric(&self, m: &Metric) {
        self.recorder.metric(m);
    }

    fn flush(&self) {
        self.recorder.flush();
    }
}

/// Sends [`Tick::Done`] on drop.
pub(super) struct Finish(Sender<Tick>);

impl Drop for Finish {
    fn drop(&mut self) {
        drop(self.0.send(Tick::Done));
    }
}

/// The watcher: journals each recorded batch as it arrives and makes the journal durable once the last flush is a
/// window old. Returns when the node is done; an append failure ends it with that error.
pub(super) fn watch(env: &Env<'_>, calls: &Calls, wake: &Receiver<Tick>) -> Result<(), PiiError> {
    let mut left = START_DURABLE_WITHIN.saturating_sub(env.jr.since_flush());
    loop {
        match wake.recv_timeout(left) {
            Ok(Tick::Recorded) => {
                journal(env, calls)?;
                left = START_DURABLE_WITHIN.saturating_sub(env.jr.since_flush());
            }
            Ok(Tick::Done) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {
                if let Err(e) = env.jr.flush_if_stale(START_DURABLE_WITHIN) {
                    env.jr.report_flush_failure(&e);
                }
                left = START_DURABLE_WITHIN;
            }
        }
    }
}

/// Appends every observation the node has recorded and not yet journalled, in order. A spawn is the only record of its
/// agent's pid, so a batch that carries one is made durable at once rather than at the window.
pub(super) fn journal(env: &Env<'_>, calls: &Calls) -> Result<(), PiiError> {
    let mut spawned = false;
    for observation in calls.take() {
        spawned |=
            matches!(&observation, RunEvent::NodeObserved { kind, .. } if kind == "coder.spawned");
        env.jr.append(&observation)?;
    }
    if spawned {
        env.jr.flush()?;
    }
    Ok(())
}
