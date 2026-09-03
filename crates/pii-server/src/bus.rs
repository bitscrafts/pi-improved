//! Journal→bus bridge (F2.4/F2.5).
//!
//! The bridge is an [`Observer`]: the Phase-1 journaler is the only redb
//! writer and notifies observers with `ObsEvent::Journal { seq, event }`;
//! this module maps those to wire [`Event`]s and broadcasts them. Reattach
//! replays the journal (`seq > since`) before attaching the live stream and
//! drops any live event whose `seq` is not greater than the last replayed —
//! the no-duplicate guarantee (F2.5).

use std::sync::Arc;

use pii_abi::{RunEvent, RunId};
use pii_core::{Metric, ObsEvent, Observer};
use pii_proto::event::{Event, WireTerminal};
use pii_proto::run::SeqEvent;
use tokio::sync::broadcast;

/// Fan-out point for wire events.
pub struct Bus {
    tx: broadcast::Sender<SeqEvent>,
}

impl Bus {
    /// A fresh bus.
    #[must_use]
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(4096);
        Arc::new(Self { tx })
    }

    /// Live subscription.
    pub fn subscribe(&self) -> broadcast::Receiver<SeqEvent> {
        self.tx.subscribe()
    }

    /// The broadcast sender (for per-run bridge observers).
    #[must_use]
    pub fn subscribe_tx(&self) -> broadcast::Sender<SeqEvent> {
        self.tx.clone()
    }

    /// Broadcasts a run cancellation (not journalled by the engine). Uses the
    /// run's current journal seq so the subscriber's dedup cursor is not
    /// poisoned (F19, opus review): a `u64::MAX` seq would permanently silence
    /// every open subscription.
    pub fn broadcast_cancelled(&self, run_id: &str, seq: u64) {
        let _ = self.tx.send(SeqEvent {
            seq,
            event: Event::RunCancelled {
                run_id: run_id.to_owned(),
            },
        });
    }
}

/// Per-run observer bridging the journal to the bus (F2.4).
pub struct BridgeObserver {
    /// The run this bridge serves.
    pub run_id: RunId,
    /// Bus sender.
    pub tx: broadcast::Sender<SeqEvent>,
}

impl Observer for BridgeObserver {
    fn event(&self, e: &ObsEvent) {
        if let ObsEvent::Journal { seq, event } = e
            && let Some(ev) = map_event(&self.run_id, event)
        {
            let _ = self.tx.send(SeqEvent {
                seq: *seq,
                event: ev,
            });
        }
    }

    fn metric(&self, _m: &Metric) {}
    fn flush(&self) {}
}

/// Maps a journalled `RunEvent` to its wire `Event`.
#[must_use]
pub fn map_event(run_id: &RunId, ev: &RunEvent) -> Option<Event> {
    Some(match ev {
        RunEvent::Started { graph, .. } => Event::RunStarted {
            run_id: run_id.to_string(),
            graph: graph.clone(),
        },
        RunEvent::TickStarted { .. } | RunEvent::Suspended { .. } | RunEvent::Resumed { .. } => {
            return None;
        }
        RunEvent::NodeStarted { node, .. } => Event::NodeStarted {
            run_id: run_id.to_string(),
            node: node.to_string(),
        },
        RunEvent::NodeFinished {
            node, port, value, ..
        } => Event::NodeFinished {
            run_id: run_id.to_string(),
            node: node.to_string(),
            port: port.to_string(),
            value: value.clone(),
        },
        RunEvent::NodeFailed {
            node,
            error,
            retryable,
            ..
        } => Event::NodeFailed {
            run_id: run_id.to_string(),
            node: node.to_string(),
            error: error.clone(),
            retryable: *retryable,
        },
        RunEvent::NodeCancelled { node } | RunEvent::NodeDead { node } => Event::NodeDead {
            run_id: run_id.to_string(),
            node: node.to_string(),
        },
        RunEvent::Finished { terminal, .. } => Event::RunFinished {
            run_id: run_id.to_string(),
            terminal: map_terminal(terminal),
        },
    })
}

fn map_terminal(t: &pii_abi::Terminal) -> WireTerminal {
    match t {
        pii_abi::Terminal::Success { output } => WireTerminal::Success {
            output: output.clone(),
        },
        pii_abi::Terminal::Failure { error } => WireTerminal::Failure {
            error: error.clone(),
        },
        pii_abi::Terminal::Suspended { key } => WireTerminal::Suspended { key: key.0.clone() },
    }
}

/// Reattach (F2.5): replay `seq > since` from the journal, then attach the
/// live receiver and drop live events with `seq <= since`. Returns the
/// merged stream in order — every event exactly once.
#[must_use]
pub fn reattach(
    store: &pii_store::Store,
    run: &RunId,
    since: u64,
    live: &mut broadcast::Receiver<SeqEvent>,
    out: &mut Vec<SeqEvent>,
) -> u64 {
    let mut last = since;
    if let Ok(journal) = store.read_journal(run) {
        for (seq, ev) in journal {
            if seq > since
                && let Some(event) = map_event(run, &ev)
            {
                out.push(SeqEvent { seq, event });
                last = seq;
            }
        }
    }
    // Live events are deduplicated against the replay cursor.
    while let Ok(se) = live.try_recv() {
        if se.seq > last {
            out.push(se.clone());
            last = se.seq;
            let _ = se;
        }
    }
    last
}
