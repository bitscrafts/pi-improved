//! Name registry: the vocabulary of event kinds, measure names and subsystems.
//!
//! Call sites and tests use these symbols, not literals. Every constant must
//! also be listed in [`ALL`]; a unit test checks the list has no duplicates
//! or empty names.

/// Metric: nodes newly marked dead.
pub const GRAPH_NODE_DEAD: &str = "graph.node.dead";
/// Metric: a join that can never be satisfied.
pub const GRAPH_JOIN_UNSATISFIABLE: &str = "graph.join.unsatisfiable";
/// Metric: width of an executed batch.
pub const GRAPH_BATCH_WIDTH: &str = "graph.batch.width";
/// Event: a durable-store write failed.
pub const STORE_PERSIST_ERROR: &str = "store.persist.error";
/// Event: a capability was denied.
pub const CORE_CAP_DENIED: &str = "core.cap.denied";
/// Event: a subsystem's health changed.
pub const HEALTH_STATE_CHANGED: &str = "health.state.changed";
/// Measure: observer events dropped.
pub const OBSERVE_EVENTS_DROPPED: &str = "observe.events.dropped";
/// Event: an egress request was denied by the net policy.
pub const NET_EGRESS_DENIED: &str = "net.egress.denied";
/// Event: an outbound dial failed.
pub const NET_DIAL_FAILED: &str = "net.dial.failed";
/// Event: accepting an inbound connection failed.
pub const NET_ACCEPT_FAILED: &str = "net.accept.failed";
/// Event: a tool was invoked (tool name and outcome).
pub const TOOLS_CALL: &str = "tools.call";
/// Event: a bash command matched the deny list.
pub const TOOLS_DENY_LIST_HIT: &str = "tools.deny_list.hit";
/// Event: the provider and model a request resolved to.
pub const MODEL_PROVIDER_IN_FORCE: &str = "model.provider.in_force";
/// Measure: model request latency.
pub const MODEL_REQUEST_LATENCY: &str = "model.request.latency";
/// Event: a model provider returned an error.
pub const MODEL_PROVIDER_ERROR: &str = "model.provider.error";
/// Event: a supervised worker changed state.
pub const WORKER_STATE_CHANGED: &str = "worker.state.changed";
/// Event: a memory search completed.
pub const MEMORY_SEARCH: &str = "memory.search";
/// Event: a memory consolidation completed.
pub const MEMORY_CONSOLIDATE: &str = "memory.consolidate";

/// Every registered name; keep in sync with the constants above.
pub const ALL: &[&str] = &[
    GRAPH_NODE_DEAD,
    GRAPH_JOIN_UNSATISFIABLE,
    GRAPH_BATCH_WIDTH,
    STORE_PERSIST_ERROR,
    CORE_CAP_DENIED,
    HEALTH_STATE_CHANGED,
    OBSERVE_EVENTS_DROPPED,
    NET_EGRESS_DENIED,
    NET_DIAL_FAILED,
    NET_ACCEPT_FAILED,
    TOOLS_CALL,
    TOOLS_DENY_LIST_HIT,
    MODEL_PROVIDER_IN_FORCE,
    MODEL_REQUEST_LATENCY,
    MODEL_PROVIDER_ERROR,
    WORKER_STATE_CHANGED,
    MEMORY_SEARCH,
    MEMORY_CONSOLIDATE,
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn registry_has_no_duplicates_and_no_empties() {
        let mut seen = std::collections::BTreeSet::new();
        for n in ALL {
            assert!(!n.is_empty(), "empty name in registry");
            assert!(seen.insert(*n), "duplicate name {n}");
        }
    }

    #[test]
    fn existing_vocabulary_is_preserved() {
        assert!(ALL.contains(&"graph.node.dead"));
        assert!(ALL.contains(&"graph.join.unsatisfiable"));
        assert!(ALL.contains(&"graph.batch.width"));
    }
}
