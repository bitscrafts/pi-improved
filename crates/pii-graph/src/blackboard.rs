//! Run blackboard (spec F4.10).

use std::collections::BTreeMap;

/// In-memory blackboard: the node-value map plus the run input.
///
/// Reads are memory-only. Writes append `NodeFinished` into the same redb
/// write transaction that advances the `RunHeader` (pass-3 §4.8). Snapshots
/// are taken every 32 journal events and unconditionally on suspend. We do
/// **not** make the blackboard per-key write-through — orki measured a 22×
/// penalty from per-key granularity, and durability, not granularity, was the
/// issue.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Blackboard {
    /// Node id -> last emitted value (on the port the node took).
    pub nodes: BTreeMap<String, serde_json::Value>,
    /// The run's root input payload.
    pub input: serde_json::Value,
}

impl Blackboard {
    /// A blackboard with the given run input.
    #[must_use]
    pub fn new(input: serde_json::Value) -> Self {
        Self {
            nodes: BTreeMap::new(),
            input,
        }
    }

    /// Records a node's emitted value.
    pub fn record(&mut self, node: &str, value: serde_json::Value) {
        self.nodes.insert(node.to_string(), value);
    }

    /// Returns the value a node produced, if any.
    #[must_use]
    pub fn get(&self, node: &str) -> Option<&serde_json::Value> {
        self.nodes.get(node)
    }

    /// Serialises the blackboard to JSON bytes (for `Store::put_snapshot`).
    ///
    /// # Errors
    /// Returns [`pii_abi::PiiError::Internal`] on a serialisation failure.
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, pii_abi::PiiError> {
        serde_json::to_vec(self)
            .map_err(|e| pii_abi::PiiError::Internal(format!("blackboard snapshot: {e}")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Blackboard;

    #[test]
    fn record_and_get() {
        let mut bb = Blackboard::new(serde_json::json!({"root": 1}));
        bb.record("a", serde_json::json!({"v": 1}));
        assert_eq!(bb.get("a"), Some(&serde_json::json!({"v": 1})));
        assert_eq!(bb.get("missing"), None);
        assert_eq!(bb.input, serde_json::json!({"root": 1}));
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut bb = Blackboard::new(serde_json::json!({}));
        bb.record("x", serde_json::json!(42));
        let bytes = bb.snapshot_bytes().unwrap();
        let back: Blackboard = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, bb);
    }
}
