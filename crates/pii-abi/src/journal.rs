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
    /// The run reached a terminal disposition.
    Finished {
        /// The terminal outcome.
        terminal: Terminal,
        /// Hash of the terminal output.
        output_hash: String,
    },
    /// A `spawn` node launched a child run (spec 32 R1): the parent→child
    /// link `pii graph trace` shows alongside the spawning node's own
    /// `NodeFinished`.
    RunSpawned {
        /// The child run's id.
        child_run: RunId,
    },
}
