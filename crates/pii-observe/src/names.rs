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

/// Every registered name; keep in sync with the constants above.
pub const ALL: &[&str] = &[
    GRAPH_NODE_DEAD,
    GRAPH_JOIN_UNSATISFIABLE,
    GRAPH_BATCH_WIDTH,
    STORE_PERSIST_ERROR,
    CORE_CAP_DENIED,
    HEALTH_STATE_CHANGED,
    OBSERVE_EVENTS_DROPPED,
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
