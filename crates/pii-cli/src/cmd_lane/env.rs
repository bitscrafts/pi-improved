//! The lane's environment, read once (spec 317 R3). `var_os` with an empty value treated as unset.

/// Every environment variable the lane reads, already decided.
#[allow(clippy::struct_excessive_bools)] // reason: each flag is one exact-value knob the shell reads (spec 317 R3, spec 320 R4)
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
    /// `PII_METHOD`: the methodology `spec.toml` routes on (spec 385); default `sdd`.
    pub(crate) method: Option<String>,
    /// `PII_SSH`: the ssh binary (default `ssh`).
    pub(crate) ssh: Option<String>,
    /// `DEVBOX_HOST`, default [`DEFAULT_HOST`].
    pub(crate) devbox_host: Option<String>,
    /// `PII_DEVBOX_CONTAINER`, default `pii-devbox`.
    pub(crate) container: Option<String>,
    /// `PII_POLL_SECONDS`, default 5.
    pub(crate) poll_seconds: Option<String>,
    /// `PII_DAEMON_SH`: the daemon script (default `modules/devbox/daemon.sh`).
    pub(crate) daemon_sh: Option<String>,
    /// `PII_STALE_BIN_OK=1` exactly: the binary-label guard is set aside (spec 320 R4).
    pub(crate) stale_bin_ok: bool,
    /// `PII_LANE_TMP_ROOT_OK=1` exactly: a `_tmp/` lane root is deliberate (spec 321 R2).
    pub(crate) tmp_root_ok: bool,
}

/// The devbox host the lane uses when `DEVBOX_HOST` is unset (the only place the literal lives).
pub(crate) const DEFAULT_HOST: &str = "mvcorrea@10.10.10.217";

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
            method: var("PII_METHOD"),
            ssh: var("PII_SSH"),
            devbox_host: var("DEVBOX_HOST"),
            container: var("PII_DEVBOX_CONTAINER"),
            poll_seconds: var("PII_POLL_SECONDS"),
            daemon_sh: var("PII_DAEMON_SH"),
            stale_bin_ok: var("PII_STALE_BIN_OK").as_deref() == Some("1"),
            tmp_root_ok: var("PII_LANE_TMP_ROOT_OK").as_deref() == Some("1"),
        }
    }
}
