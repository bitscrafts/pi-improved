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
    /// Re-run one node of a recorded run (spec 186).
    #[serde(rename = "run.retry")]
    RunRetry,
    /// Close a run as failed with a reason (spec 186).
    #[serde(rename = "run.abandon")]
    RunAbandon,
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
    /// List pending approvals.
    #[serde(rename = "approve.list")]
    ApproveList,
    /// Resolve an approval (allow/deny).
    #[serde(rename = "approve.resolve")]
    ApproveResolve,
    /// List heal-attempt ledger entries (S0.4).
    #[serde(rename = "heal.list")]
    HealList,
    /// Fetch one heal-attempt entry by signature hash (S0.4).
    #[serde(rename = "heal.get")]
    HealGet,
    /// List cron entries.
    #[serde(rename = "cron.list")]
    CronList,
    /// List configured channels.
    #[serde(rename = "channel.list")]
    ChannelList,
    /// Memory search.
    #[serde(rename = "memory.search")]
    MemorySearch,
    /// Memory put.
    #[serde(rename = "memory.put")]
    MemoryPut,
    /// Memory consolidate.
    #[serde(rename = "memory.consolidate")]
    MemoryConsolidate,
    /// Reload config (channels/workers), never restarts the process.
    #[serde(rename = "sys.reload")]
    SysReload,
    /// Promote a verified learning candidate; the daemon applies and records it (spec 138).
    #[serde(rename = "learn.promote")]
    LearnPromote,
    /// Take a promotion back by hand; the daemon applies and records it (spec 138).
    #[serde(rename = "learn.revert")]
    LearnRevert,
    /// One promotion lifecycle pass (TTL, outcome windows); the daemon records it (spec 138).
    #[serde(rename = "learn.sweep")]
    LearnSweep,
    /// Add a skill document to the daemon's store (spec 233 R1).
    #[serde(rename = "skill.put")]
    SkillPut,
    /// List the skill documents in the daemon's store (spec 233 R2).
    #[serde(rename = "skill.list")]
    SkillList,
    /// Change a live skill document's text in the daemon's store, keeping its id (spec 234 R1).
    #[serde(rename = "skill.edit")]
    SkillEdit,
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
            Self::RunRetry => "run.retry",
            Self::RunAbandon => "run.abandon",
            Self::RunTrace => "run.trace",
            Self::GraphApply => "graph.apply",
            Self::GraphGet => "graph.get",
            Self::GraphList => "graph.list",
            Self::GraphLint => "graph.lint",
            Self::EventSubscribe => "event.subscribe",
            Self::EventUnsubscribe => "event.unsubscribe",
            Self::WorkerList => "worker.list",
            Self::WorkerReload => "worker.reload",
            Self::ApproveList => "approve.list",
            Self::ApproveResolve => "approve.resolve",
            Self::HealList => "heal.list",
            Self::HealGet => "heal.get",
            Self::CronList => "cron.list",
            Self::ChannelList => "channel.list",
            Self::MemorySearch => "memory.search",
            Self::MemoryPut => "memory.put",
            Self::MemoryConsolidate => "memory.consolidate",
            Self::SysReload => "sys.reload",
            Self::LearnPromote => "learn.promote",
            Self::LearnRevert => "learn.revert",
            Self::LearnSweep => "learn.sweep",
            Self::SkillPut => "skill.put",
            Self::SkillList => "skill.list",
            Self::SkillEdit => "skill.edit",
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
            "run.retry" => Self::RunRetry,
            "run.abandon" => Self::RunAbandon,
            "run.trace" => Self::RunTrace,
            "graph.apply" => Self::GraphApply,
            "graph.get" => Self::GraphGet,
            "graph.list" => Self::GraphList,
            "graph.lint" => Self::GraphLint,
            "event.subscribe" => Self::EventSubscribe,
            "event.unsubscribe" => Self::EventUnsubscribe,
            "worker.list" => Self::WorkerList,
            "worker.reload" => Self::WorkerReload,
            "approve.list" => Self::ApproveList,
            "approve.resolve" => Self::ApproveResolve,
            "heal.list" => Self::HealList,
            "heal.get" => Self::HealGet,
            "cron.list" => Self::CronList,
            "channel.list" => Self::ChannelList,
            "memory.search" => Self::MemorySearch,
            "memory.put" => Self::MemoryPut,
            "memory.consolidate" => Self::MemoryConsolidate,
            "sys.reload" => Self::SysReload,
            "learn.promote" => Self::LearnPromote,
            "learn.revert" => Self::LearnRevert,
            "learn.sweep" => Self::LearnSweep,
            "skill.put" => Self::SkillPut,
            "skill.list" => Self::SkillList,
            "skill.edit" => Self::SkillEdit,
            other => return Err(other.to_owned()),
        })
    }
}
