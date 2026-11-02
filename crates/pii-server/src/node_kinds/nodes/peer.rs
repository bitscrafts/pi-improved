//! Spec 343 R5: a node finds its peers inside its OWN namespace.
//!
//! A graph is written once and used twice: standalone, and inlined by a `call` (spec 255), where every node id is
//! namespaced — `record` becomes `queue__record`. A node that looks up its peer by the literal `select` therefore
//! finds nothing once it is inside a call, and the failure is silent: `None` reads as "no item was selected".
//!
//! The engine hands every node its own id in the config as `node-id` (`pii_graph::node_id::with_node_id`), so the
//! peer id is derivable: take the namespace this node lives in and keep the peer's own name.
//!
//! A suffix search (`any key ending in __select`) would also work today and would match ANOTHER call's `select` in
//! a graph that calls two queues. Deriving from the namespace cannot: a node only ever sees its own neighbours.

/// The blackboard id of the peer called `name`, resolved in this node's namespace.
///
/// `queue__record` + `select` → `queue__select`; `record` + `select` → `select`.
#[must_use]
pub fn peer_of(cfg: &serde_json::Value, name: &str) -> String {
    cfg.get("node-id")
        .and_then(serde_json::Value::as_str)
        .and_then(|own| own.rsplit_once("__"))
        .map_or_else(
            || name.to_owned(),
            |(prefix, _)| format!("{prefix}__{name}"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_node_finds_a_bare_peer() {
        let cfg = serde_json::json!({ "node-id": "record" });
        assert_eq!(peer_of(&cfg, "select"), "select");
    }

    #[test]
    fn a_namespaced_node_finds_a_namespaced_peer() {
        let cfg = serde_json::json!({ "node-id": "queue__record" });
        assert_eq!(peer_of(&cfg, "select"), "queue__select");
    }

    #[test]
    fn a_nested_call_keeps_the_whole_prefix() {
        // `night` calls `queue`, which calls `retry`: the innermost prefix is the whole path
        let cfg = serde_json::json!({ "node-id": "sdd__retry__record" });
        assert_eq!(peer_of(&cfg, "select"), "sdd__retry__select");
    }

    #[test]
    fn no_node_id_falls_back_to_the_bare_name() {
        // a node built outside the engine (a unit test, a tool) must not panic or invent a prefix
        let cfg = serde_json::json!({});
        assert_eq!(peer_of(&cfg, "select"), "select");
    }
}
