//! Node input/output contract and the synchronous metadata trait.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::effects::Effects;
use crate::failover::FailoverReason;
use crate::ids::PortName;

/// Input handed to a node when it is scheduled.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[doc(hidden)]
pub struct NodeInput {
    /// The run's root input payload.
    pub input: serde_json::Value,
    /// The join payload: a JSON object keyed by source node id holding the
    /// value of every `Fired` inbound Data edge. A node with no inbound Data
    /// edges gets `input.clone()` here (set by the scheduler, not the ABI).
    pub payload: serde_json::Value,
    /// The blackboard's node-value map, keyed by node id.
    pub nodes: serde_json::Value,
}

/// Payload emitted on a node's `fail` port for a classified provider
/// failure (A3). Distinct from the free-form `{"error": String}` shape
/// the `err` port already uses for non-provider failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FailurePayload {
    /// HTTP status if the provider gave one (`None` for e.g. a local
    /// context-overflow with no HTTP leg, matching `ProviderFailure::status`).
    pub status: Option<u16>,
    /// The classified reason as its `Debug`/`serde` tag — reuses
    /// `FailoverReason`'s own derive, not a re-stringified copy.
    pub reason: FailoverReason,
    /// `ProviderFailure::provider` (e.g. `"ollama"`, `"llamacpp"`), passed
    /// through unchanged.
    pub provider: String,
}

/// Result of a node's execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[doc(hidden)]
pub enum Outcome {
    /// Continue on the named output port with a value.
    Next {
        /// The port to follow.
        port: PortName,
        /// The value emitted on that port.
        value: serde_json::Value,
    },
    /// The pipeline suspended and must later be resumed.
    Suspend {
        /// The reason the run suspended (approval, timer, event, input).
        reason: SuspendReason,
        /// The resume key under which the run may be resumed.
        key: ResumeKey,
    },
    /// The pipeline failed with a message.
    Fail {
        /// Human-readable failure description.
        error: String,
    },
}

/// Why a run suspended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[doc(hidden)]
pub enum SuspendReason {
    /// Waiting for human approval of a policy.
    Approval {
        /// The policy that needs approving.
        policy: String,
        /// The prompt shown to the approver.
        prompt: String,
    },
    /// Waiting until a timer elapses.
    Timer {
        /// Delay in milliseconds.
        in_ms: u64,
    },
    /// Waiting for an event on a topic.
    Event {
        /// The topic name.
        topic: String,
    },
    /// Waiting for human input on a channel.
    HumanInput {
        /// The channel name.
        channel: String,
        /// The prompt shown to the human.
        prompt: String,
    },
}

/// Opaque handle used to resume a suspended run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[doc(hidden)]
pub struct ResumeKey(pub String);

/// Synchronous metadata about a node (pass-3 §4.1).
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

    use super::{FailurePayload, NodeInput, Outcome, ResumeKey, SuspendReason};
    use crate::failover::{FailoverReason, Origin, classify};
    use crate::ids::PortName;

    #[test]
    fn failure_payload_roundtrips() {
        // Construct the reason via classify on a real 429 body so the test also
        // proves `FailurePayload.reason`'s field type lines up with a
        // `FailoverReason` the classifier actually produces.
        let reason = classify(Some(429), "Too Many Requests", Origin::Provider);
        assert_eq!(reason, FailoverReason::RateLimit);
        let original = FailurePayload {
            status: Some(429),
            reason,
            provider: String::from("ollama"),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: FailurePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn outcome_serde_roundtrip() {
        let original = Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({"x": 1}),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Outcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn suspend_serde_roundtrip() {
        let original = Outcome::Suspend {
            reason: SuspendReason::Timer { in_ms: 500 },
            key: ResumeKey(String::from("k1")),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Outcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn node_input_payload_roundtrip() {
        let input = NodeInput {
            input: serde_json::json!({"root": 1}),
            payload: serde_json::json!({"k": 1}),
            nodes: serde_json::json!({"a": 2}),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: NodeInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.payload, input.payload);
        assert_eq!(parsed.nodes, input.nodes);
    }
}
