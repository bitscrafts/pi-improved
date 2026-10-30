//! The lane's environment, read once (spec 317 R3). `var_os` with an empty value treated as unset.

/// Every environment variable the lane reads, already decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaneEnv {
    /// `ALLOW_DIRTY`: any non-empty value allows a dirty tree.
    pub(crate) allow_dirty: bool,
    /// `PII_LANE_GRAPH=spec` exactly selects the spec lane.
    pub(crate) spec_lane: bool,
    /// `PII_GRAPH_EDIT_OK=1` exactly (dry runs only).
    pub(crate) graph_edit_ok: bool,
    pub(crate) agent: Option<String>,
    pub(crate) claude_model: Option<String>,
    pub(crate) pi_model: Option<String>,
    pub(crate) pi_provider: Option<String>,
    /// Raw text, as the shell interpolates it.
    pub(crate) max_budget: Option<String>,
    pub(crate) node_timeout_ms: Option<String>,
    pub(crate) wait_margin_s: Option<String>,
}

fn var(name: &str) -> Option<String> {
    std::env::var_os(name)
        .and_then(|v| v.into_string().ok())
        .filter(|v| !v.is_empty())
}

impl LaneEnv {
    /// Reads the process environment once.
    pub(crate) fn read() -> Self {
        Self {
            allow_dirty: var("ALLOW_DIRTY").is_some(),
            spec_lane: var("PII_LANE_GRAPH").as_deref() == Some("spec"),
            graph_edit_ok: var("PII_GRAPH_EDIT_OK").as_deref() == Some("1"),
            agent: var("PII_AGENT"),
            claude_model: var("CLAUDE_MODEL"),
            pi_model: var("PI_MODEL"),
            pi_provider: var("PI_PROVIDER"),
            max_budget: var("PII_MAX_BUDGET_USD"),
            node_timeout_ms: var("PII_NODE_TIMEOUT_MS"),
            wait_margin_s: var("PII_WAIT_MARGIN_S"),
        }
    }
}
