//! Event-sourced journal types shared by the store and the graph engine.
//!
//! These live in `pii-abi` (not `pii-store` or `pii-graph`) because both of
//! those crates need them, and `pii-store` must never depend on `pii-graph`.
//! All time fields are milliseconds relative to run start, measured with
//! [`std::time::Instant`] so journal replay is deterministic.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{NodeId, PortName, RunId};
use crate::node::{ResumeKey, SuspendReason};

/// Lifecycle status of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum RunStatus {
    /// The run is actively executing (or waiting to be resumed).
    Running,
    /// The run suspended and awaits a resume key.
    Suspended,
    /// The run completed successfully.
    Succeeded,
    /// The run failed.
    Failed,
    /// The run's agent call was refused before any spawn (spec 221): it would not start, which is not a failure.
    Refused,
}

/// Durable header describing one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunHeader {
    /// Run identifier.
    pub run: RunId,
    /// Graph name (or body hash) this run executes.
    pub graph: String,
    /// Current lifecycle status.
    pub status: RunStatus,
    /// Next journal sequence number to be written.
    pub seq: u64,
    /// Wall-clock duration in milliseconds (0 while running).
    pub dur_ms: u32,
    /// Build revision of the binary that accepted the run; empty on rows
    /// written before spec 188 (counted as unattributed, never as a deploy).
    #[serde(default)]
    pub revision: String,
    /// Epoch ms at which the accepting process started (one value per process).
    #[serde(default)]
    pub process_start_ms: u64,
    /// Epoch ms at which the run was accepted.
    #[serde(default)]
    pub created_ms: u64,
}

/// What the running process is: the origin every run it accepts is stamped with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOrigin {
    /// Build revision of the running binary.
    pub revision: String,
    /// Epoch ms at which this process started.
    pub process_start_ms: u64,
}

/// Terminal disposition of a finished run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Terminal {
    /// The run succeeded, carrying its final output.
    Success {
        /// The terminal output JSON.
        output: serde_json::Value,
    },
    /// The run failed with a message.
    Failure {
        /// Human-readable failure description.
        error: String,
    },
    /// The run suspended and can be resumed by key.
    Suspended {
        /// The resume key.
        key: ResumeKey,
    },
}

/// One event in a run's journal — the append-only source of truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RunEvent {
    /// The run started.
    Started {
        /// The graph name.
        graph: String,
        /// Hash of the run input.
        input_hash: String,
    },
    /// A scheduling tick drained a batch of ready nodes.
    TickStarted {
        /// The nodes drained into this batch (in declaration order).
        batch: Vec<NodeId>,
        /// Number of nodes in the batch.
        width: u32,
    },
    /// A node began executing.
    NodeStarted {
        /// The node id.
        node: NodeId,
        /// Millisecond offset of dispatch.
        at_ms: u32,
    },
    /// A node completed on a port.
    NodeFinished {
        /// The node id.
        node: NodeId,
        /// The port it took.
        port: PortName,
        /// The value it produced.
        value: serde_json::Value,
        /// Hash of the value.
        value_hash: String,
        /// Millisecond offset of dispatch.
        at_ms: u32,
        /// Execution duration in milliseconds.
        dur_ms: u32,
    },
    /// A node failed.
    NodeFailed {
        /// The node id.
        node: NodeId,
        /// Failure description.
        error: String,
        /// Whether the failure is retryable.
        retryable: bool,
        /// 1-based attempt number.
        attempt: u32,
    },
    /// A heartbeat from a node that can block on a process: it is alive and
    /// has been running `elapsed_ms`, its child having used `child_cpu_ms` of
    /// CPU. Journal/audit only — `replay` treats it as a no-op — so a wedged
    /// node (elapsed grows, cpu flat) reads differently from a working one.
    NodeProgress {
        /// The node id.
        node: NodeId,
        /// Milliseconds since the node began executing.
        elapsed_ms: u64,
        /// CPU milliseconds consumed by the node's child process.
        child_cpu_ms: u64,
    },
    /// A node was cancelled (not failed).
    NodeCancelled {
        /// The node id.
        node: NodeId,
    },
    /// A node was killed by dead-branch propagation (never ran).
    NodeDead {
        /// The node id.
        node: NodeId,
    },
    /// Audit marker: on resume, a `NodeStarted` with no later terminal event
    /// (`NodeFinished`/`NodeFailed`/`NodeCancelled`/`NodeDead`) was found — the
    /// previous attempt left this node mid-flight. Purely journal/audit: it
    /// carries NO state transition and `replay` treats it as a no-op (the node
    /// still re-runs, exactly as if it were absent).
    NodeInterrupted {
        /// The node id that was left mid-flight.
        node: NodeId,
    },
    /// A node suspended the run.
    Suspended {
        /// The node id.
        node: NodeId,
        /// The reason for suspension.
        reason: SuspendReason,
        /// The resume key.
        key: ResumeKey,
    },
    /// The run was resumed after suspension.
    Resumed {
        /// The resume key.
        key: ResumeKey,
        /// Hash of the resumed input payload.
        payload_hash: String,
    },
    /// An observation a node made about an agent call (spec 217), recorded by the engine from the node's own
    /// events: `coder.spawned`, `coder.exited`, or `worker.start.refused`. The fields are the event's, never the
    /// node's returned value.
    NodeObserved {
        /// The node that made the observation.
        node: NodeId,
        /// The observed event's kind, e.g. `coder.spawned`.
        kind: String,
        /// The observed event's fields.
        fields: serde_json::Value,
    },
    /// The run reached a terminal disposition.
    Finished {
        /// The terminal outcome.
        terminal: Terminal,
        /// Hash of the terminal output.
        output_hash: String,
    },
    /// An effect node (`post`, `memory put`, `spawn`, `hal`) is about to act:
    /// its idempotency key, journaled and made durable BEFORE the effect
    /// (spec 186). A key started and never confirmed means the effect may or
    /// may not have happened — replay reports `effect.unknown`.
    EffectStarted {
        /// The effect node.
        node: NodeId,
        /// Idempotency key: hash of (run id, node id, input digest).
        key: String,
    },
    /// The effect node's effect is done; carries the output so a replay that
    /// crashed before `NodeFinished` reuses it instead of acting again.
    EffectConfirmed {
        /// The effect node.
        node: NodeId,
        /// The key recorded by the matching `EffectStarted`.
        key: String,
        /// The port the node took.
        port: PortName,
        /// The value it produced.
        value: serde_json::Value,
    },
    /// Operator action (`pii run retry`): the node's earlier terminal events
    /// are history; the node runs again. The journal is append-only, so replay
    /// honours this marker rather than the journal being rewritten.
    NodeRetried {
        /// The node being re-run.
        node: NodeId,
    },
    /// A `spawn` node launched a child run (spec 32 R1): the parent→child
    /// link `pii graph trace` shows alongside the spawning node's own
    /// `NodeFinished`.
    RunSpawned {
        /// The child run's id.
        child_run: RunId,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::RunHeader;

    #[test]
    fn an_old_header_still_decodes() {
        let old = r#"{"run":"r0001","graph":"g","status":"Succeeded","seq":3,"dur_ms":9}"#;
        let h: RunHeader = serde_json::from_str(old).unwrap();
        assert_eq!(h.revision, "");
        assert_eq!(h.process_start_ms, 0);
        assert_eq!(h.created_ms, 0);
        assert_eq!(h.seq, 3);
    }
}
