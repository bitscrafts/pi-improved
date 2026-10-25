//! Spec 271 R1: a node's factory is built knowing its own id. The id rides in the built config as `node-id`; the spec's
//! own cfg, its hash and the journal never carry it.

use pii_abi::NodeId;

/// The node's config with `node-id` set to `id`, unless the config is not a table or already names one.
#[must_use]
pub fn with_node_id(cfg: &serde_json::Value, id: &NodeId) -> serde_json::Value {
    let mut built = cfg.clone();
    if let serde_json::Value::Object(map) = &mut built {
        map.entry("node-id")
            .or_insert_with(|| serde_json::Value::String(id.as_str().to_owned()));
    }
    built
}
