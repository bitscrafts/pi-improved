//! Observability: events, metrics, and observer multiplexing.

use std::sync::Arc;

use parking_lot::Mutex;

/// An observation about the engine's execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsEvent {
    /// A journal event was appended (unsequenced form, for callers that
    /// emit events outside a run journal).
    Run(pii_abi::RunEvent),
    /// A journal event was appended at the given sequence number. Emitted by
    /// the run engine so observers can stream `{"seq":n,"event":..}` lines.
    Journal {
        /// The journal sequence the event was written at.
        seq: u64,
        /// The appended event.
        event: pii_abi::RunEvent,
    },
    /// A capability denial occurred.
    Denial {
        /// The principal that was denied.
        principal: String,
        /// The capability that was denied.
        cap: String,
    },
    /// A tool call finished (spec 108): reported by `ToolSet::observe_call`
    /// through the run's observer so the daemon, whose tool sets carry no
    /// observer of their own, still logs it. `deny_entry` names the bash
    /// deny-list entry the command matched, if any.
    ToolCall {
        /// The tool name.
        tool: String,
        /// `ok` / `denied` / `error`.
        outcome: String,
        /// The matched bash deny-list entry.
        deny_entry: Option<String>,
    },
    /// The model tier an agent node resolved to (spec 114): the run's record of
    /// which tier and which model spec it used.
    TierResolved {
        /// The tier name.
        tier: String,
        /// The tier's model spec.
        model: String,
    },
    /// The memory search an agent turn runs to inject context failed (spec 109
    /// R4). The turn continues without memory; this is how the failure stays
    /// visible. Never carries the query text.
    MemorySearchFailed {
        /// The error text.
        error: String,
    },
    /// A node panicked (spec 135); the run fails with the message, the
    /// daemon survives.
    NodePanicked {
        /// The run the node belonged to.
        run: String,
        /// The node id.
        node: String,
        /// The panic message.
        message: String,
    },
    /// Boot reconciliation closed a run a crash left `Running` (spec 154):
    /// one event per reconciled run, naming it and why.
    RunReconciled {
        /// The reconciled run.
        run: String,
        /// Why it was closed.
        reason: String,
    },
    /// An agent-call observation a node made (spec 217): `coder.spawned`, `coder.exited` or
    /// `worker.start.refused`. The engine records it into the run journal as `RunEvent::NodeObserved`.
    NodeObserved {
        /// The observed event's kind.
        kind: String,
        /// The observed event's fields.
        fields: serde_json::Value,
    },
    /// A durable-store write failed outside the run journal (D3/D6, 026
    /// report 5.0.4) — e.g. a session append/snapshot. The Phase-1
    /// journaler has its own `Result` return for callers that check it
    /// directly; this variant exists for call sites (like session
    /// persistence) that are best-effort by design and must not panic or
    /// abort on a write failure, but also must not silently swallow it.
    PersistError {
        /// What was being persisted (e.g. `"session:<id>"`).
        context: String,
        /// The error text.
        message: String,
    },
}

/// A named numeric metric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metric {
    /// Metric name.
    pub name: String,
    /// Metric value.
    pub value: u64,
}

/// Subscriber to engine events and metrics.
pub trait Observer: Send + Sync + 'static {
    /// Called with each observation.
    fn event(&self, e: &ObsEvent);
    /// Called with each metric update.
    fn metric(&self, m: &Metric);
    /// Flushes any buffered output.
    fn flush(&self);
}

/// An observer that ignores everything.
pub struct NullObserver;

impl Observer for NullObserver {
    fn event(&self, _e: &ObsEvent) {}
    fn metric(&self, _m: &Metric) {}
    fn flush(&self) {}
}

/// An in-memory observer that records everything it is told.
pub struct RecordingObserver {
    metrics: Mutex<Vec<Metric>>,
    events: Mutex<Vec<ObsEvent>>,
}

impl RecordingObserver {
    /// A fresh observer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            metrics: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    /// All recorded metrics.
    #[must_use]
    pub fn metrics(&self) -> Vec<Metric> {
        self.metrics.lock().clone()
    }

    /// The maximum value recorded for a metric name (0 if never set).
    #[must_use]
    pub fn max_metric(&self, name: &str) -> u64 {
        self.metrics
            .lock()
            .iter()
            .filter(|m| m.name == name)
            .map(|m| m.value)
            .max()
            .unwrap_or(0)
    }

    /// All recorded events.
    #[must_use]
    pub fn events(&self) -> Vec<ObsEvent> {
        self.events.lock().clone()
    }
}

impl Default for RecordingObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer for RecordingObserver {
    fn event(&self, e: &ObsEvent) {
        self.events.lock().push(e.clone());
    }

    fn metric(&self, m: &Metric) {
        self.metrics.lock().push(m.clone());
    }

    fn flush(&self) {}
}

/// Observes by fanning out to several observers.
pub struct MultiObserver(Vec<Arc<dyn Observer>>);

impl MultiObserver {
    /// A multiplexer over the given observers.
    #[must_use]
    pub fn new(observers: Vec<Arc<dyn Observer>>) -> Self {
        Self(observers)
    }
}

impl Observer for MultiObserver {
    fn event(&self, e: &ObsEvent) {
        for o in &self.0 {
            o.event(e);
        }
    }

    fn metric(&self, m: &Metric) {
        for o in &self.0 {
            o.metric(m);
        }
    }

    fn flush(&self) {
        for o in &self.0 {
            o.flush();
        }
    }
}

/// Bridges the legacy [`Observer`] surface onto the `pii-observe` contract.
///
/// Metrics and persist/denial events a subsystem raises through a [`crate::Ctx`]
/// reach the contract observer as named [`pii_observe::Event`]s. Metric names
/// that are not registered in [`pii_observe::names`] are dropped (an event's
/// kind must be a registered `&'static str`).
pub struct ContractObserver(Arc<dyn pii_observe::Observer>);

impl ContractObserver {
    /// Forwards to `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn pii_observe::Observer>) -> Self {
        Self(inner)
    }
}

impl Observer for ContractObserver {
    fn event(&self, e: &ObsEvent) {
        use pii_observe::{Event, Level, names};
        match e {
            ObsEvent::Denial { principal, cap } => self.0.event(
                &Event::new("core", Level::Warn, names::CORE_CAP_DENIED)
                    .field("principal", principal.as_str())
                    .field("cap", cap.as_str()),
            ),
            ObsEvent::ToolCall {
                tool,
                outcome,
                deny_entry,
            } => {
                self.0.event(
                    &Event::new("tools", Level::Info, names::TOOLS_CALL)
                        .field("tool", tool.as_str())
                        .field("outcome", outcome.as_str()),
                );
                if let Some(entry) = deny_entry {
                    self.0.event(
                        &Event::new("tools", Level::Warn, names::TOOLS_DENY_LIST_HIT)
                            .field("entry", entry.as_str()),
                    );
                }
            }
            ObsEvent::TierResolved { tier, model } => self.0.event(
                &Event::new("model", Level::Info, names::MODEL_TIER_RESOLVED)
                    .field("tier", tier.as_str())
                    .field("model", model.as_str()),
            ),
            ObsEvent::PersistError { context, message } => self.0.event(
                &Event::new("store", Level::Error, names::STORE_PERSIST_ERROR)
                    .field("context", context.as_str())
                    .field("message", message.as_str()),
            ),
            ObsEvent::MemorySearchFailed { error } => self.0.event(
                &Event::new("memory", Level::Error, names::MEMORY_SEARCH)
                    .field("outcome", "error")
                    .field("error", error.as_str())
                    .field("where", "agent_turn"),
            ),
            ObsEvent::NodePanicked { run, node, message } => self.0.event(
                &Event::new("graph", Level::Error, names::NODE_PANICKED)
                    .field("run", run.as_str())
                    .field("node", node.as_str())
                    .field("message", message.as_str())
                    .in_run(pii_observe::RunId(run.clone())),
            ),
            ObsEvent::RunReconciled { run, reason } => self.0.event(
                &Event::new("graph", Level::Warn, names::RUN_RECONCILED)
                    .field("run", run.as_str())
                    .field("reason", reason.as_str())
                    .in_run(pii_observe::RunId(run.clone())),
            ),
            // The coder logs its own event to the contract observer; the journal copy is the engine's (spec 217).
            ObsEvent::Run(_) | ObsEvent::Journal { .. } | ObsEvent::NodeObserved { .. } => {}
        }
    }

    fn metric(&self, m: &Metric) {
        use pii_observe::{Event, Level, names};
        if let Some(kind) = names::ALL.iter().find(|n| **n == m.name) {
            self.0
                .event(&Event::new("graph", Level::Debug, kind).field("value", m.value));
        }
    }

    fn flush(&self) {
        self.0.flush();
    }
}

/// Forwards ONLY capability/tool denials to a contract observer.
///
/// [`ContractObserver`] also forwards metrics; the audit trail wants the
/// denials alone, so it cannot be flooded by per-tick graph metrics.
pub struct DenialObserver(Arc<dyn pii_observe::Observer>);

impl DenialObserver {
    /// Forwards denials to `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn pii_observe::Observer>) -> Self {
        Self(inner)
    }
}

impl Observer for DenialObserver {
    fn event(&self, e: &ObsEvent) {
        if matches!(e, ObsEvent::Denial { .. }) {
            ContractObserver::new(Arc::clone(&self.0)).event(e);
        }
    }

    fn metric(&self, _m: &Metric) {}

    fn flush(&self) {
        self.0.flush();
    }
}

/// Forwards ONLY tool-call events to a contract observer (spec 108).
///
/// The daemon log wants `tools.call` / `tools.deny_list.hit` for every call
/// (and a failed agent-turn memory search) but not the per-tick graph metrics [`ContractObserver`] also carries.
pub struct ToolCallObserver(Arc<dyn pii_observe::Observer>);

impl ToolCallObserver {
    /// Forwards tool calls to `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn pii_observe::Observer>) -> Self {
        Self(inner)
    }
}

impl Observer for ToolCallObserver {
    fn event(&self, e: &ObsEvent) {
        if matches!(
            e,
            ObsEvent::ToolCall { .. }
                | ObsEvent::MemorySearchFailed { .. }
                | ObsEvent::TierResolved { .. }
                | ObsEvent::NodePanicked { .. }
                | ObsEvent::RunReconciled { .. }
        ) {
            ContractObserver::new(Arc::clone(&self.0)).event(e);
        }
    }

    fn metric(&self, _m: &Metric) {}

    fn flush(&self) {
        self.0.flush();
    }
}

/// Environment variable that turns the trajectory sink on (a directory).
pub const TRAJECTORY_DIR_ENV: &str = "PII_TRAJECTORY_DIR";
/// Environment variable listing field names the sink drops (comma-separated).
pub const TRAJECTORY_REDACT_ENV: &str = "PII_TRAJECTORY_REDACT";

/// The trajectory sink, if enabled.
///
/// `PII_TRAJECTORY_DIR` wins over
/// `config_dir` (the daemon config's `trajectory_dir`). Default OFF: with
/// neither set this returns `None`. `PII_TRAJECTORY_REDACT` names fields the
/// sink drops. Write failures are reported to stderr.
#[must_use]
pub fn trajectory_sink(config_dir: Option<&str>) -> Option<Arc<pii_observe::TrajectorySink>> {
    trajectory_sink_with(
        config_dir,
        DEFAULT_TRAJECTORY_RETENTION,
        Arc::new(pii_observe::StderrObserver),
    )
}

/// Default aggregate bound on the trajectory directory (spec 115): 512 MiB
/// and 50 000 files, whichever is hit first.
pub const DEFAULT_TRAJECTORY_RETENTION: pii_observe::Retention = pii_observe::Retention {
    max_bytes: Some(512 * 1024 * 1024),
    max_files: Some(50_000),
};

/// [`trajectory_sink`] with an explicit retention policy and the observer its
/// prune/truncate/write-failure events go to (the daemon passes its log).
#[must_use]
pub fn trajectory_sink_with(
    config_dir: Option<&str>,
    retention: pii_observe::Retention,
    reporter: Arc<dyn pii_observe::Observer>,
) -> Option<Arc<pii_observe::TrajectorySink>> {
    let dir = crate::env_var(TRAJECTORY_DIR_ENV)
        .filter(|d| !d.is_empty())
        .or_else(|| config_dir.filter(|d| !d.is_empty()).map(str::to_owned))?;
    let policy: Arc<dyn pii_observe::RedactionPolicy> = crate::env_var(TRAJECTORY_REDACT_ENV)
        .and_then(|l| pii_observe::DropFields::parse(&l))
        .map_or_else(
            || Arc::new(pii_observe::NoRedaction) as Arc<dyn pii_observe::RedactionPolicy>,
            |p| Arc::new(p),
        );
    let policy: Arc<dyn pii_observe::RedactionPolicy> = Arc::new(
        pii_observe::ScrubbingPolicy::new(policy, crate::secret_scrubber()),
    );
    Some(Arc::new(
        pii_observe::TrajectorySink::new(dir, policy)
            .with_retention(retention)
            .with_reporter(reporter),
    ))
}

/// Feeds a run's journal to the trajectory sink (spec 103).
///
/// One per run: it stamps every record with the run (and session) id,
/// forwards each journal event as a `run.journal` event, and — when the
/// journal records the terminal — also a `run.outcome` event carrying the
/// terminal verbatim (`Failure` stays `Failure`; nothing is re-derived).
pub struct TrajectoryBridge {
    sink: Arc<dyn pii_observe::Observer>,
    run: pii_observe::RunId,
    session: Option<pii_observe::SessionId>,
}

impl TrajectoryBridge {
    /// A bridge for `run` (and optionally `session`) into `sink`.
    #[must_use]
    pub fn new(
        sink: Arc<dyn pii_observe::Observer>,
        run: &pii_abi::RunId,
        session: Option<&str>,
    ) -> Self {
        Self {
            sink,
            run: pii_observe::RunId(run.to_string()),
            session: session.map(|s| pii_observe::SessionId(s.to_owned())),
        }
    }

    fn stamp(&self, e: pii_observe::Event) -> pii_observe::Event {
        let e = e.in_run(self.run.clone());
        match &self.session {
            Some(s) => e.in_session(s.clone()),
            None => e,
        }
    }

    /// The outcome a terminal maps to, verbatim.
    #[must_use]
    pub fn outcome_of(t: &pii_abi::Terminal) -> pii_observe::Outcome {
        use pii_observe::{Outcome, Verdict};
        match t {
            pii_abi::Terminal::Success { output } => Outcome {
                verdict: Verdict::Success,
                reason: String::from("terminal"),
                detail: output.clone(),
            },
            pii_abi::Terminal::Failure { error } => Outcome {
                verdict: Verdict::Failure,
                reason: String::from("terminal"),
                detail: serde_json::Value::String(error.clone()),
            },
            pii_abi::Terminal::Suspended { key } => Outcome {
                verdict: Verdict::Suspended,
                reason: String::from("terminal"),
                detail: serde_json::Value::String(key.0.clone()),
            },
        }
    }

    /// Records a cancelled run (cancellation is not journalled by the engine).
    pub fn cancelled(&self, reason: &str) {
        let o = pii_observe::Outcome {
            verdict: pii_observe::Verdict::Cancelled,
            reason: reason.to_owned(),
            detail: serde_json::Value::Null,
        };
        self.sink.event(&self.stamp(o.into_event("graph")));
    }
}

impl Observer for TrajectoryBridge {
    fn event(&self, e: &ObsEvent) {
        use pii_observe::{Event, Level, names};
        let ObsEvent::Journal { seq, event } = e else {
            return;
        };
        let body = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        self.sink.event(
            &self.stamp(
                Event::new("graph", Level::Info, names::RUN_JOURNAL)
                    .field("seq", *seq)
                    .field("event", body),
            ),
        );
        if let pii_abi::RunEvent::Finished { terminal, .. } = event {
            self.sink
                .event(&self.stamp(Self::outcome_of(terminal).into_event("graph")));
        }
    }

    fn metric(&self, _m: &Metric) {}

    fn flush(&self) {
        self.sink.flush();
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::sync::Arc;

    use super::{
        ContractObserver, DenialObserver, Metric, ObsEvent, Observer, RecordingObserver,
        TrajectoryBridge,
    };

    #[test]
    fn run_tap_forwards_a_failed_memory_search_as_memory_search_error() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let o = super::ToolCallObserver::new(rec.clone());
        o.event(&ObsEvent::MemorySearchFailed {
            error: "index gone".into(),
        });
        o.event(&ObsEvent::PersistError {
            context: "c".into(),
            message: "m".into(),
        });
        let ev = rec.events_named(pii_observe::names::MEMORY_SEARCH);
        assert_eq!(ev.len(), 1, "only the memory failure passes the tap");
        assert_eq!(
            ev[0].fields.get("outcome").and_then(|v| v.as_str()),
            Some("error")
        );
        assert_eq!(
            ev[0].fields.get("error").and_then(|v| v.as_str()),
            Some("index gone")
        );
    }

    #[test]
    fn run_tap_forwards_the_resolved_tier_as_model_tier_resolved() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let o = super::ToolCallObserver::new(rec.clone());
        o.event(&ObsEvent::TierResolved {
            tier: "fast".into(),
            model: "scripted:a.json".into(),
        });
        let ev = rec.events_named(pii_observe::names::MODEL_TIER_RESOLVED);
        assert_eq!(ev.len(), 1);
        assert_eq!(
            ev[0].fields.get("tier").and_then(|v| v.as_str()),
            Some("fast")
        );
        assert_eq!(
            ev[0].fields.get("model").and_then(|v| v.as_str()),
            Some("scripted:a.json")
        );
    }

    #[test]
    fn denial_observer_forwards_denials_and_nothing_else() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let o = DenialObserver::new(rec.clone());
        o.event(&ObsEvent::Denial {
            principal: "p".into(),
            cap: "fs_write".into(),
        });
        o.event(&ObsEvent::PersistError {
            context: "c".into(),
            message: "m".into(),
        });
        o.metric(&Metric {
            name: "graph.batch.width".into(),
            value: 2,
        });
        let kinds: Vec<_> = rec.events().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [pii_observe::names::CORE_CAP_DENIED]);
    }

    #[test]
    fn recording_observer_reports_max_metric() {
        let o = Arc::new(RecordingObserver::new());
        for v in [1u64, 2, 1] {
            o.metric(&Metric {
                name: String::from("graph.batch.width"),
                value: v,
            });
        }
        assert_eq!(o.max_metric("graph.batch.width"), 2);
        assert_eq!(o.max_metric("missing"), 0);
    }

    #[test]
    fn contract_observer_maps_registered_metrics_and_drops_unknown() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let o = ContractObserver::new(rec.clone());
        o.metric(&Metric {
            name: String::from(pii_observe::names::GRAPH_NODE_DEAD),
            value: 2,
        });
        o.metric(&Metric {
            name: String::from("not.registered"),
            value: 1,
        });
        let evs = rec.events();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, pii_observe::names::GRAPH_NODE_DEAD);
        assert_eq!(evs[0].fields.get("value"), Some(&serde_json::json!(2)));
    }

    fn finished(t: pii_abi::Terminal) -> ObsEvent {
        ObsEvent::Journal {
            seq: 9,
            event: pii_abi::RunEvent::Finished {
                terminal: t,
                output_hash: String::new(),
            },
        }
    }

    #[test]
    fn trajectory_bridge_records_a_failed_terminal_verbatim_as_failure() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let b = TrajectoryBridge::new(rec.clone(), &pii_abi::RunId::from("r1"), Some("s1"));
        b.event(&finished(pii_abi::Terminal::Failure {
            error: String::from("exit 7"),
        }));
        let out = rec.events_named(pii_observe::names::RUN_OUTCOME);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].fields.get("verdict"),
            Some(&serde_json::json!("failure"))
        );
        assert_eq!(
            out[0].fields.get("detail"),
            Some(&serde_json::json!("exit 7"))
        );
        assert_eq!(out[0].run, Some(pii_observe::RunId("r1".into())));
        assert_eq!(out[0].session, Some(pii_observe::SessionId("s1".into())));
    }

    #[test]
    fn trajectory_bridge_success_emits_success_and_no_failure() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let b = TrajectoryBridge::new(rec.clone(), &pii_abi::RunId::from("r1"), None);
        b.event(&finished(pii_abi::Terminal::Success {
            output: serde_json::json!({"text": "hi"}),
        }));
        let out = rec.events_named(pii_observe::names::RUN_OUTCOME);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].fields.get("verdict"),
            Some(&serde_json::json!("success"))
        );
        assert_eq!(
            out[0].fields.get("detail"),
            Some(&serde_json::json!({"text": "hi"}))
        );
    }

    #[test]
    fn trajectory_bridge_non_terminal_journal_event_has_no_outcome() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let b = TrajectoryBridge::new(rec.clone(), &pii_abi::RunId::from("r1"), None);
        b.event(&ObsEvent::Journal {
            seq: 1,
            event: pii_abi::RunEvent::Started {
                graph: String::from("g"),
                input_hash: String::new(),
            },
        });
        assert_eq!(rec.events_named(pii_observe::names::RUN_JOURNAL).len(), 1);
        assert!(rec.events_named(pii_observe::names::RUN_OUTCOME).is_empty());
    }

    #[test]
    fn trajectory_bridge_cancelled_records_cancelled() {
        let rec = Arc::new(pii_observe::Recorder::new());
        let b = TrajectoryBridge::new(rec.clone(), &pii_abi::RunId::from("r1"), None);
        b.cancelled("cancel");
        let out = rec.events_named(pii_observe::names::RUN_OUTCOME);
        assert_eq!(
            out[0].fields.get("verdict"),
            Some(&serde_json::json!("cancelled"))
        );
    }

    #[test]
    fn trajectory_sink_is_off_by_default_and_on_with_a_config_dir() {
        if crate::env_var(super::TRAJECTORY_DIR_ENV).is_some() {
            return; // the operator's env wins; nothing to assert about "off"
        }
        assert!(super::trajectory_sink(None).is_none());
        assert!(super::trajectory_sink(Some("")).is_none());
        assert!(super::trajectory_sink(Some("/nonexistent-root")).is_some());
    }
}
