//! Identity and tracing context for capability decisions.

use pii_abi::{NodeId, RunId};

/// Who is acting. Every capability decision is attributed to a principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// An interactive user.
    User,
    /// A graph node executing within a run.
    GraphNode {
        /// The run id.
        run: RunId,
        /// The node id.
        node: NodeId,
    },
    /// A plugin.
    Plugin(String),
    /// An external channel.
    Channel(String),
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::GraphNode { run, node } => write!(f, "node:{node}@run:{run}"),
            Self::Plugin(p) => write!(f, "plugin:{p}"),
            Self::Channel(c) => write!(f, "channel:{c}"),
        }
    }
}

/// Trace context for correlating journal/diagnostic output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCtx {
    /// The run this trace belongs to.
    pub run: RunId,
    /// A monotonically increasing span number (incremented on `child`).
    pub span: u64,
}
