//! The closed method vocabulary (F1.3). Unknown methods surface as
//! `MethodNotFound`, never as a value.

use serde::{Deserialize, Serialize};

/// Every wire method, exactly as on the wire (no `Other` escape hatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    /// Hello handshake.
    #[serde(rename = "sys.hello")]
    SysHello,
    /// Server status.
    #[serde(rename = "sys.status")]
    SysStatus,
    /// Server version.
    #[serde(rename = "sys.version")]
    SysVersion,
    /// Shut the daemon down.
    #[serde(rename = "sys.shutdown")]
    SysShutdown,
    /// Create a session.
    #[serde(rename = "session.create")]
    SessionCreate,
    /// Prompt a session.
    #[serde(rename = "session.prompt")]
    SessionPrompt,
    /// Interrupt the session's active run.
    #[serde(rename = "session.interrupt")]
    SessionInterrupt,
    /// Session snapshot.
    #[serde(rename = "session.state")]
    SessionState,
    /// List sessions.
    #[serde(rename = "session.list")]
    SessionList,
    /// Close a session.
    #[serde(rename = "session.close")]
    SessionClose,
    /// Start a graph run.
    #[serde(rename = "run.start")]
    RunStart,
    /// Run header.
    #[serde(rename = "run.get")]
    RunGet,
    /// List runs.
    #[serde(rename = "run.list")]
    RunList,
    /// Cancel a run.
    #[serde(rename = "run.cancel")]
    RunCancel,
    /// Resume a suspended run.
    #[serde(rename = "run.resume")]
    RunResume,
    /// Run journal trace.
    #[serde(rename = "run.trace")]
    RunTrace,
    /// Store/replace a graph.
    #[serde(rename = "graph.apply")]
    GraphApply,
    /// Fetch a graph.
    #[serde(rename = "graph.get")]
    GraphGet,
    /// List graphs.
    #[serde(rename = "graph.list")]
    GraphList,
    /// Lint a graph source.
    #[serde(rename = "graph.lint")]
    GraphLint,
    /// Subscribe to events.
    #[serde(rename = "event.subscribe")]
    EventSubscribe,
    /// Unsubscribe.
    #[serde(rename = "event.unsubscribe")]
    EventUnsubscribe,
    /// List workers and their state.
    #[serde(rename = "worker.list")]
    WorkerList,
    /// Reload a worker (drain → respawn).
    #[serde(rename = "worker.reload")]
    WorkerReload,
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SysHello => "sys.hello",
            Self::SysStatus => "sys.status",
            Self::SysVersion => "sys.version",
            Self::SysShutdown => "sys.shutdown",
            Self::SessionCreate => "session.create",
            Self::SessionPrompt => "session.prompt",
            Self::SessionInterrupt => "session.interrupt",
            Self::SessionState => "session.state",
            Self::SessionList => "session.list",
            Self::SessionClose => "session.close",
            Self::RunStart => "run.start",
            Self::RunGet => "run.get",
            Self::RunList => "run.list",
            Self::RunCancel => "run.cancel",
            Self::RunResume => "run.resume",
            Self::RunTrace => "run.trace",
            Self::GraphApply => "graph.apply",
            Self::GraphGet => "graph.get",
            Self::GraphList => "graph.list",
            Self::GraphLint => "graph.lint",
            Self::EventSubscribe => "event.subscribe",
            Self::EventUnsubscribe => "event.unsubscribe",
            Self::WorkerList => "worker.list",
            Self::WorkerReload => "worker.reload",
        })
    }
}

impl std::str::FromStr for Method {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "sys.hello" => Self::SysHello,
            "sys.status" => Self::SysStatus,
            "sys.version" => Self::SysVersion,
            "sys.shutdown" => Self::SysShutdown,
            "session.create" => Self::SessionCreate,
            "session.prompt" => Self::SessionPrompt,
            "session.interrupt" => Self::SessionInterrupt,
            "session.state" => Self::SessionState,
            "session.list" => Self::SessionList,
            "session.close" => Self::SessionClose,
            "run.start" => Self::RunStart,
            "run.get" => Self::RunGet,
            "run.list" => Self::RunList,
            "run.cancel" => Self::RunCancel,
            "run.resume" => Self::RunResume,
            "run.trace" => Self::RunTrace,
            "graph.apply" => Self::GraphApply,
            "graph.get" => Self::GraphGet,
            "graph.list" => Self::GraphList,
            "graph.lint" => Self::GraphLint,
            "event.subscribe" => Self::EventSubscribe,
            "event.unsubscribe" => Self::EventUnsubscribe,
            "worker.list" => Self::WorkerList,
            "worker.reload" => Self::WorkerReload,
            other => return Err(other.to_owned()),
        })
    }
}
