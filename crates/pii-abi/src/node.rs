//! Node input/output contract and the synchronous metadata trait.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::effects::Effects;
use crate::ids::PortName;

/// Input handed to a node when it is scheduled.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[doc(hidden)]
pub struct NodeInput {
    /// Arbitrary JSON payload produced by upstream nodes.
    pub payload: serde_json::Value,
}

/// Result of a node's execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[doc(hidden)]
pub enum Outcome {
    /// Continue on the named output port.
    Next {
        /// The port to follow.
        port: PortName,
    },
    /// The pipeline finished successfully.
    Done,
    /// The pipeline failed with a message.
    Failed {
        /// Human-readable failure description.
        error: String,
    },
}

/// Synchronous metadata about a node (pass-3 §4.1). The async `run`
/// contract arrives in Phase 1 with the widened allowlist.
pub trait NodeMeta {
    /// Stable kind string identifying the node implementation.
    fn kind(&self) -> &str;
    /// What external state this node touches.
    fn effects(&self) -> Effects;
    /// Output ports this node may take.
    fn ports(&self) -> &[PortName];
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{NodeInput, Outcome};
    use crate::ids::PortName;

    #[test]
    fn outcome_serde_roundtrip() {
        let original = Outcome::Next {
            port: PortName::from("ok"),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Outcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn node_input_payload_roundtrip() {
        let input = NodeInput {
            payload: serde_json::json!({"k": 1}),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: NodeInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.payload, input.payload);
    }
}
