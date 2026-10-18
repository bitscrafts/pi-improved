//! Effect ledger (spec 186): idempotency keys and the replay scan over them.
//!
//! An effect node journals [`RunEvent::EffectStarted`] (durably) before it
//! acts and [`RunEvent::EffectConfirmed`] after. A replay reads the ledger:
//! a confirmed effect is never repeated, a started-but-unconfirmed one makes
//! the replay refuse with `effect.unknown`.

use std::collections::BTreeMap;

use pii_abi::{NodeId, NodeInput, PiiError, PortName, RunEvent, RunId};

/// Digest of the input an effect node acts on (its join payload).
#[must_use]
pub fn input_digest(input: &NodeInput) -> String {
    serde_json::to_vec(&input.payload).map_or_else(|_| String::new(), |b| pii_store::hash(&b))
}

/// Idempotency key: hash of (run id, node id, input digest). The same node on
/// a different input is a different key, so it runs again.
#[must_use]
pub fn idempotency_key(run: &RunId, node: &NodeId, input_digest: &str) -> String {
    pii_store::hash(format!("{run}\0{node}\0{input_digest}").as_bytes())
}

/// What a journal says about its effect nodes.
#[derive(Debug, Default)]
pub struct Ledger {
    /// Effects started and never resolved: node → key.
    pub unknown: BTreeMap<NodeId, String>,
    /// Effects confirmed whose `NodeFinished` never landed (a crash in the
    /// gap): node → (port, value), reused instead of acting again.
    pub orphans: BTreeMap<NodeId, (PortName, serde_json::Value)>,
}

/// Scans a journal. A node's own report (`NodeFinished`, `NodeFailed`,
/// `NodeCancelled`, `Suspended`) resolves its open effect; so does a later
/// `NodeRetried` (the operator accepted the risk).
#[must_use]
pub fn scan(journal: &[(u64, RunEvent)]) -> Ledger {
    let mut open: BTreeMap<NodeId, String> = BTreeMap::new();
    let mut orphans: BTreeMap<NodeId, (PortName, serde_json::Value)> = BTreeMap::new();
    for (_, ev) in journal {
        match ev {
            RunEvent::EffectStarted { node, key } => {
                open.insert(node.clone(), key.clone());
                orphans.remove(node);
            }
            RunEvent::EffectConfirmed {
                node, port, value, ..
            } => {
                open.remove(node);
                orphans.insert(node.clone(), (port.clone(), value.clone()));
            }
            RunEvent::NodeFinished { node, .. }
            | RunEvent::NodeFailed { node, .. }
            | RunEvent::NodeCancelled { node }
            | RunEvent::Suspended { node, .. }
            | RunEvent::NodeRetried { node } => {
                open.remove(node);
                orphans.remove(node);
            }
            _ => {}
        }
    }
    Ledger {
        unknown: open,
        orphans,
    }
}

/// The refusal for a ledger with unresolved effects; `None` when clean.
#[must_use]
pub fn refusal(ledger: &Ledger) -> Option<PiiError> {
    if ledger.unknown.is_empty() {
        return None;
    }
    let nodes: Vec<String> = ledger
        .unknown
        .iter()
        .map(|(n, k)| format!("{n} (key {k})"))
        .collect();
    Some(PiiError::Invalid(format!(
        "{}: started and never confirmed, so it is unknown whether it happened: {}; \
         `pii run retry --node <id>` accepts the risk, `pii run abandon` closes the run",
        pii_observe::names::EFFECT_UNKNOWN,
        nodes.join(", ")
    )))
}

/// Journal view with operator retries applied: for each `NodeRetried`, the
/// node's earlier events are history, and so is every earlier `NodeDead`
/// (dead propagation is recomputed from the edges).
#[must_use]
pub fn effective(journal: &[(u64, RunEvent)]) -> Vec<(u64, RunEvent)> {
    let touches = |ev: &RunEvent, n: &NodeId| match ev {
        RunEvent::NodeFinished { node, .. }
        | RunEvent::NodeFailed { node, .. }
        | RunEvent::NodeCancelled { node }
        | RunEvent::NodeDead { node }
        | RunEvent::NodeStarted { node, .. }
        | RunEvent::NodeProgress { node, .. }
        | RunEvent::NodeInterrupted { node }
        | RunEvent::Suspended { node, .. } => node == n,
        _ => false,
    };
    let mut out: Vec<(u64, RunEvent)> = Vec::with_capacity(journal.len());
    for (seq, ev) in journal {
        if let RunEvent::NodeRetried { node } = ev {
            out.retain(|(_, e)| !touches(e, node) && !matches!(e, RunEvent::NodeDead { .. }));
        }
        out.push((*seq, ev.clone()));
    }
    out
}
