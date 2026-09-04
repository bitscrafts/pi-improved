//! Daemon config (F5.2): one TOML file, `deny_unknown_fields` throughout —
//! a typo'd key must be a load error, not a silently-ignored no-op.
//!
//! **Complete env surface for this phase** (N10, every read goes through
//! [`pii_core::env_var`]): `PII_CONFIG`, `PII_RUNTIME_DIR`, `PII_MODEL`,
//! `PII_MEMORY_PATH`, `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`,
//! `PII_DISCORD_API_BASE`, `OLLAMA_HOST`, `OLLAMA_API_KEY`.

use pii_abi::PiiError;
use serde::Deserialize;

/// `[net]`: egress policy knobs (F2.2).
///
/// `grants` is a list of `host:port` patterns granted to every principal —
/// this phase does not need per-principal partitioning (`pii-net`'s own
/// suite covers that at the policy-engine level).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct NetSection {
    /// `host:port` patterns every principal may reach.
    pub grants: Vec<String>,
    /// Whether loopback/private/link-local classes are allowed.
    pub allow_private: bool,
}

/// `[[worker]]` (F3.3 fields, reused verbatim).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerSection {
    /// Worker name (grant/rpc key).
    pub name: String,
    /// Program to spawn.
    pub program: String,
    /// argv after the program.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra child environment.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Concurrent in-flight cap (0 = default).
    #[serde(default)]
    pub max_in_flight: u32,
}

/// `[[channel]]` (F5.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSection {
    /// Channel name.
    pub name: String,
    /// Channel kind (`discord`, `cli`).
    pub kind: String,
    /// Graph to run per inbound message.
    pub graph: String,
    /// Poll interval in milliseconds.
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,
    /// `cli` kind only: the inbox file path.
    #[serde(default)]
    pub inbox: Option<String>,
    /// `cli` kind only: the outbox file path.
    #[serde(default)]
    pub outbox: Option<String>,
}

const fn default_poll_ms() -> u64 {
    500
}

/// `[memory]` (F5.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MemorySection {
    /// Memory store path (empty = `<runtime_dir>/memory.redb`).
    pub path: String,
    /// Embedder name (reserved; only `trigram` ships this phase).
    pub embedder: String,
    /// Cache budget in bytes (reserved).
    pub cache_bytes: u64,
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            path: String::new(),
            embedder: String::from("trigram"),
            cache_bytes: 0,
        }
    }
}

/// `[[cron]]` (F5.2/F5.8).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronSection {
    /// Cron entry name.
    pub name: String,
    /// Graph to run (its entry must be a `timer` node carrying the cron
    /// expression: F5.8 drives firing from the graph, not this section).
    pub graph: String,
    /// Display-only schedule mirror for `cron.list` (F5.2).
    #[serde(default)]
    pub schedule: String,
    /// Extra run input merged under `input`.
    #[serde(default)]
    pub input: serde_json::Value,
}

/// The whole daemon config file (F5.2). An unknown top-level key is a load
/// error (`deny_unknown_fields`): a typo'd grant must not silently become
/// no grant.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// `[net]`.
    pub net: NetSection,
    /// `[[worker]]`.
    pub worker: Vec<WorkerSection>,
    /// `[[channel]]`.
    pub channel: Vec<ChannelSection>,
    /// `[memory]`.
    pub memory: MemorySection,
    /// `[[cron]]`.
    pub cron: Vec<CronSection>,
}

impl Config {
    /// Loads and parses `path`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a read or parse failure (including any
    /// unknown key).
    pub fn load(path: &std::path::Path) -> Result<Self, PiiError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| PiiError::Invalid(format!("config {}: {e}", path.display())))?;
        Self::parse(&raw)
    }

    /// Parses TOML text directly (tested without a filesystem).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a parse failure (including any unknown key).
    pub fn parse(raw: &str) -> Result<Self, PiiError> {
        toml::from_str(raw).map_err(|e| PiiError::Invalid(format!("config: {e}")))
    }

    /// The default config path: `<runtime_dir>/pii.toml` when present, else
    /// an empty config (F5.2). `PII_CONFIG` overrides the path outright.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a parse failure.
    pub fn load_default(runtime_dir: &std::path::Path) -> Result<Self, PiiError> {
        if let Some(p) = pii_core::env_var("PII_CONFIG") {
            return Self::load(std::path::Path::new(&p));
        }
        let default_path = runtime_dir.join("pii.toml");
        if default_path.exists() {
            return Self::load(&default_path);
        }
        Ok(Self::default())
    }
}

/// A [`pii_net::grant::GrantStore`] giving every principal the same grant.
///
/// Scoped by the config's `grants` host patterns (F5.2): this phase needs
/// no per-principal partitioning — `pii-net`'s own suite covers isolation
/// at the policy-engine level.
pub struct ConfigGrants {
    connect: pii_net::grant::HostScope,
    allow_private: bool,
}

impl ConfigGrants {
    /// Builds from the `[net]` section.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for a malformed host pattern.
    pub fn from_section(net: &NetSection) -> Result<Self, PiiError> {
        Ok(Self {
            connect: pii_net::grant::HostScope::parse_all(&net.grants)?,
            allow_private: net.allow_private,
        })
    }

    fn one_grant(&self) -> pii_net::grant::NetGrant {
        pii_net::grant::NetGrant {
            connect: self.connect.clone(),
            allow_private: self.allow_private,
            max_conns: 64,
            max_bytes_out: 16 * 1024 * 1024,
            max_bytes_in: 16 * 1024 * 1024,
            expires_at_ms: None,
        }
    }
}

impl pii_net::grant::GrantStore for ConfigGrants {
    fn grant(&self, _principal: &str, _key: &str) -> Option<pii_net::grant::NetGrant> {
        Some(self.one_grant())
    }

    fn all(&self) -> Vec<(String, String, pii_net::grant::NetGrant)> {
        vec![(String::from("*"), String::from("*"), self.one_grant())]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Config;

    #[test]
    fn parses_a_full_config() {
        let raw = r#"
[net]
grants = ["127.0.0.1:*"]
allow_private = true

[[worker]]
name = "w1"
program = "/bin/pii"
args = ["worker-exec"]

[[channel]]
name = "discord1"
kind = "discord"
graph = "starter/assistant"
poll_interval_ms = 250

[memory]
path = "mem.redb"

[[cron]]
name = "consolidate"
graph = "starter/consolidate"
schedule = "*/15 * * * *"
"#;
        let cfg = Config::parse(raw).unwrap();
        assert!(cfg.net.allow_private);
        assert_eq!(cfg.worker.len(), 1);
        assert_eq!(cfg.channel.len(), 1);
        assert_eq!(cfg.channel.first().unwrap().poll_interval_ms, 250);
        assert_eq!(cfg.cron.len(), 1);
    }

    #[test]
    fn unknown_key_is_an_error() {
        let raw = "[net]\nbogus = true\n";
        let err = Config::parse(raw).unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Invalid(_)));
    }

    #[test]
    fn empty_config_parses() {
        let cfg = Config::parse("").unwrap();
        assert!(cfg.worker.is_empty());
        assert!(cfg.channel.is_empty());
        assert!(cfg.cron.is_empty());
    }
}
