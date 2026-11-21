//! The trajectory sink: where a run's journal is written, and the bridge that
//! feeds it (specs 103 and 115).

use std::sync::Arc;

use super::{Metric, ObsEvent, Observer};

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
