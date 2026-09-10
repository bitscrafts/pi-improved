//! Server→client push vocabulary (F1.7): the ONLY events a client can
//! receive. Closed enum, `kind`-tagged `snake_case`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Terminal disposition of a run (wire copy of `pii_abi::Terminal`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireTerminal {
    /// The run succeeded.
    Success {
        /// Terminal output JSON.
        output: serde_json::Value,
    },
    /// The run failed.
    Failure {
        /// Failure description.
        error: String,
    },
    /// The run suspended.
    Suspended {
        /// Resume key.
        key: String,
    },
}

/// The complete push vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A run began.
    RunStarted {
        /// Run id.
        run_id: String,
        /// Graph name.
        graph: String,
    },
    /// A node started executing.
    NodeStarted {
        /// Run id.
        run_id: String,
        /// Node id.
        node: String,
    },
    /// A node completed on a port.
    NodeFinished {
        /// Run id.
        run_id: String,
        /// Node id.
        node: String,
        /// Port taken.
        port: String,
        /// Value produced.
        value: serde_json::Value,
    },
    /// A node failed.
    NodeFailed {
        /// Run id.
        run_id: String,
        /// Node id.
        node: String,
        /// Error text.
        error: String,
        /// Whether the failure is retryable.
        retryable: bool,
    },
    /// A node was killed by dead propagation.
    NodeDead {
        /// Run id.
        run_id: String,
        /// Node id.
        node: String,
    },
    /// A streaming assistant text chunk.
    AssistantDelta {
        /// Run id.
        run_id: String,
        /// Session id.
        session_id: String,
        /// Text chunk.
        text: String,
    },
    /// A tool call began.
    ToolCall {
        /// Run id.
        run_id: String,
        /// Node id.
        node: String,
        /// Tool name.
        tool: String,
        /// Call arguments.
        args: serde_json::Value,
    },
    /// A tool call finished.
    ToolResult {
        /// Run id.
        run_id: String,
        /// Node id.
        node: String,
        /// Tool name.
        tool: String,
        /// Whether the tool succeeded.
        ok: bool,
        /// Short summary.
        summary: String,
    },
    /// A run reached a terminal disposition.
    RunFinished {
        /// Run id.
        run_id: String,
        /// Terminal outcome.
        terminal: WireTerminal,
    },
    /// A run was cancelled.
    RunCancelled {
        /// Run id.
        run_id: String,
    },
    /// A run suspended (gate/timer) and awaits a resume key.
    RunSuspended {
        /// Run id.
        run_id: String,
        /// The resume key.
        key: String,
        /// Human-readable reason.
        reason: String,
    },
    /// A suspended run was resumed.
    RunResumed {
        /// Run id.
        run_id: String,
        /// The resume key used.
        key: String,
    },
    /// A gate requested an approval decision.
    ApprovalRequested {
        /// Approval id.
        approval_id: String,
        /// Run id.
        run_id: String,
        /// Policy name.
        policy: String,
        /// Prompt for the approver.
        prompt: String,
    },
    /// An approval decision was recorded.
    ApprovalResolved {
        /// Approval id.
        approval_id: String,
        /// `true` = allow, `false` = deny.
        allow: bool,
    },
    /// A worker died (supervisor event).
    WorkerDied {
        /// Worker name.
        worker: String,
        /// Dead pid.
        pid: u32,
    },
    /// A worker was respawned.
    WorkerRestarted {
        /// Worker name.
        worker: String,
        /// Fresh pid.
        pid: u32,
        /// Death-to-hello milliseconds.
        took_ms: u64,
    },
    /// A channel message produced a run.
    ChannelMessage {
        /// Channel name.
        channel: String,
        /// Chat id.
        chat: String,
        /// Run id.
        run_id: String,
    },
    /// Memory search degraded (vector lane down).
    MemoryDegraded {
        /// The lanes that still ran.
        lanes: Vec<String>,
        /// Why the lane was skipped.
        reason: String,
    },
    /// A model-routing decision: which step resolved to which model
    /// (spec 28 R3 — the journal is the audit trail).
    RouteDecision {
        /// Run id.
        run_id: String,
        /// The logical step name (e.g. `patch`).
        step: String,
        /// The resolved model spec.
        model: String,
    },
    /// A `spawn` node launched a child run (spec 32 R1).
    RunSpawned {
        /// The parent run id.
        run_id: String,
        /// The child run id.
        child_run: String,
    },
    /// A subscriber fell behind and events were dropped (never silent).
    Lagged {
        /// Subscription id.
        sub_id: String,
        /// Number of events dropped.
        dropped: u64,
    },
}

/// `event.subscribe` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventFilter {
    /// Restrict to one run.
    pub run: Option<String>,
    /// Restrict to one session.
    pub session: Option<String>,
    /// Restrict to these `kind` strings.
    pub kinds: Vec<String>,
    /// Replay everything with `seq > since_seq` before going live.
    pub since_seq: Option<u64>,
}

/// `event.subscribe` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventSubscribeResult {
    /// Subscription id.
    pub sub_id: String,
}

/// The push notification params (method `event.subscribe`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventPush {
    /// Subscription id.
    pub sub_id: String,
    /// Journal sequence of the event.
    pub seq: u64,
    /// The event.
    pub event: Event,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Event, WireTerminal};

    #[test]
    fn event_roundtrip() {
        let events = vec![
            Event::RunStarted {
                run_id: "r1".into(),
                graph: "g".into(),
            },
            Event::NodeStarted {
                run_id: "r1".into(),
                node: "a".into(),
            },
            Event::NodeFinished {
                run_id: "r1".into(),
                node: "a".into(),
                port: "ok".into(),
                value: serde_json::json!(1),
            },
            Event::NodeFailed {
                run_id: "r1".into(),
                node: "a".into(),
                error: "boom".into(),
                retryable: false,
            },
            Event::NodeDead {
                run_id: "r1".into(),
                node: "b".into(),
            },
            Event::AssistantDelta {
                run_id: "r1".into(),
                session_id: "s1".into(),
                text: "hi".into(),
            },
            Event::ToolCall {
                run_id: "r1".into(),
                node: "a".into(),
                tool: "fs_read".into(),
                args: serde_json::json!({"path": "x"}),
            },
            Event::ToolResult {
                run_id: "r1".into(),
                node: "a".into(),
                tool: "fs_read".into(),
                ok: true,
                summary: "ok".into(),
            },
            Event::RunFinished {
                run_id: "r1".into(),
                terminal: WireTerminal::Success {
                    output: serde_json::json!({"text": "hi"}),
                },
            },
            Event::RunCancelled {
                run_id: "r1".into(),
            },
            Event::Lagged {
                sub_id: "sub-1".into(),
                dropped: 3,
            },
        ];
        for ev in events {
            let line = serde_json::to_string(&ev).expect("ser");
            assert!(line.contains("\"kind\":\""), "kind tag missing: {line}");
            let back: Event = serde_json::from_str(&line).expect("de");
            assert_eq!(back, ev);
        }
    }

    #[test]
    fn wire_terminal_tags() {
        let t = WireTerminal::Failure {
            error: "cancelled".into(),
        };
        assert_eq!(
            serde_json::to_string(&t).expect("ser"),
            r#"{"kind":"failure","error":"cancelled"}"#
        );
    }
}
