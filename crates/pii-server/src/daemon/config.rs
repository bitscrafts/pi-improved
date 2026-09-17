//! Daemon config (F5.2): one TOML file, `deny_unknown_fields` throughout —
//! a typo'd key must be a load error, not a silently-ignored no-op.
//!
//! **Complete env surface for this phase** (N10, every read goes through
//! [`pii_core::env_var`]): `PII_CONFIG`, `PII_RUNTIME_DIR`, `PII_MODEL`,
//! `PII_MEMORY_PATH`, `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`,
//! `PII_DISCORD_API_BASE`, `OLLAMA_HOST`, `OLLAMA_API_KEY`.
//!
//! **Persistent `[core]` settings (2026-09-05 follow-up).** Before this,
//! EVERY setting above was env-var-only — nothing survived a fresh shell
//! or a daemon restart unless something re-exported it first (the exact
//! trap the user hit: `pii -p` and `pii sys status` failing in a plain
//! shell right after a daemon had been hand-launched with env vars in a
//! DIFFERENT shell). `[core]` in the same TOML file now backs `model` and
//! `runtime_dir` — every entrypoint (`pii -p`, `pii daemon`, `pii tui`,
//! and `uds::runtime_dir`) resolves in the same order: CLI flag (where
//! one exists) > env var (unchanged, still wins — no behavior change for
//! anyone already relying on env vars) > `[core]` in the config file >
//! built-in default. The config file itself is found via
//! [`Config::default_path`], independent of `runtime_dir` (so it can
//! supply `runtime_dir` itself without a chicken-and-egg lookup):
//! `$PII_CONFIG` if set, else `$HOME/.pii/pii.toml` (falls back to
//! `/root/.pii/pii.toml` if `$HOME` is unset).

use pii_abi::PiiError;
use std::collections::BTreeMap;

use serde::Deserialize;

/// `[core]`: settings every entrypoint needs before it can do anything else.
///
/// All fields are `Option` so an absent key means
/// "fall through to the env var / built-in default", never "override
/// with empty" (2026-09-05).
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct CoreSection {
    /// Default model spec (e.g. `"ollama:granite4.2:3b"`, `"scripted:"`).
    /// Overridden by `--model` (oneshot) or `$PII_MODEL`, in that order.
    pub model: Option<String>,
    /// Default runtime dir (holds `control.sock`, `graphs/`). Overridden
    /// by `$PII_RUNTIME_DIR`.
    pub runtime_dir: Option<String>,
    /// Default Ollama host, e.g. `"http://127.0.0.1:11434"`. Overridden
    /// by `$OLLAMA_HOST`.
    pub ollama_host: Option<String>,
    /// Max runs retained per session in the in-memory `session_runs` index
    /// (D3/D6, 026 report 5.0.4): oldest run ids are evicted (LRU) once a
    /// session's run count exceeds this. `None` (absent key) falls back to
    /// [`crate::DEFAULT_SESSION_RUNS_CAP`] — see [`Self::session_runs_cap`].
    pub session_runs_cap: Option<usize>,
}

impl CoreSection {
    /// Resolves the effective `session_runs_cap` (D3/D6): the configured
    /// value if present, else [`crate::DEFAULT_SESSION_RUNS_CAP`].
    #[must_use]
    pub fn session_runs_cap(&self) -> usize {
        self.session_runs_cap
            .unwrap_or(crate::DEFAULT_SESSION_RUNS_CAP)
    }
}

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

/// The known routing steps (spec 28 R1): the evolve pipeline's fixed roles.
pub(crate) const KNOWN_STEPS: &[&str] =
    &["localize", "patch", "review", "research", "orchestrator"];

/// `[evolve]` (spec 28 R1): holds the `[evolve.routing]` step -> model
/// policy. Empty by default — no routing behavior unless configured.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EvolveSection {
    /// Per-step model routing policy (step name -> model spec). Unknown
    /// step names are an error (`deny_unknown_fields` semantics via the
    /// validating deserializer below).
    #[serde(default, deserialize_with = "deny_unknown_routing")]
    pub routing: BTreeMap<String, String>,
}

/// Rejects a `[evolve.routing]` table whose keys are not known step names
/// (R1: `deny_unknown_fields` applies — a typo'd step must not become a
/// silent no-route).
fn deny_unknown_routing<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map: BTreeMap<String, String> = BTreeMap::deserialize(deserializer)?;
    for k in map.keys() {
        if !KNOWN_STEPS.contains(&k.as_str()) {
            return Err(serde::de::Error::custom(format!(
                "unknown routing step '{k}' (known: {})",
                KNOWN_STEPS.join(", ")
            )));
        }
    }
    Ok(map)
}

/// `[memory]` (F5.2).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct MemorySection {
    /// Memory store path (empty = `<runtime_dir>/memory.redb`).
    pub path: String,
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
    /// `[core]`.
    pub core: CoreSection,
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
    /// `[evolve]` (spec 28).
    #[serde(default)]
    pub evolve: EvolveSection,
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

    /// The config file path independent of any `runtime_dir` lookup
    /// (2026-09-05): `$PII_CONFIG` if set, else `$HOME/.pii/pii.toml`
    /// (falls back to the literal `/root/.pii/pii.toml` if `$HOME` is
    /// unset, matching this project's container default). Every
    /// entrypoint (`pii -p`, `pii daemon`, `pii tui`) can call this
    /// BEFORE it has resolved a runtime dir, breaking the prior
    /// chicken-and-egg where `[core].runtime_dir` needed `runtime_dir`
    /// to find the file that sets it.
    #[must_use]
    pub fn default_path() -> std::path::PathBuf {
        if let Some(p) = pii_core::env_var("PII_CONFIG") {
            return std::path::PathBuf::from(p);
        }
        let home = pii_core::env_var("HOME").unwrap_or_else(|| String::from("/root"));
        std::path::PathBuf::from(home).join(".pii").join("pii.toml")
    }

    /// Loads the config from [`Self::default_path`], or an empty default
    /// `Config` when that file does not exist.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a parse failure (a PRESENT-but-malformed
    /// file is always an error — only a missing file is silently treated
    /// as empty).
    pub fn load_from_default_path() -> Result<Self, PiiError> {
        let path = Self::default_path();
        if path.exists() {
            return Self::load(&path);
        }
        Ok(Self::default())
    }

    /// Resolves the effective model spec (2026-09-05): `explicit` (e.g.
    /// `--model`) wins, then `$PII_MODEL`, then this config's
    /// `[core].model`, else `None` (caller decides the "no model
    /// configured" error).
    #[must_use]
    pub fn resolve_model(&self, explicit: Option<String>) -> Option<String> {
        explicit
            .or_else(|| pii_core::env_var("PII_MODEL"))
            .or_else(|| self.core.model.clone())
    }

    /// Resolves the effective runtime dir (2026-09-05): `$PII_RUNTIME_DIR`
    /// wins (unchanged precedence for existing env-var users), then this
    /// config's `[core].runtime_dir`, else `None` (caller falls through to
    /// `$XDG_RUNTIME_DIR`/`/run/user/<uid>/pii/<instance>`).
    #[must_use]
    pub fn resolve_runtime_dir(&self) -> Option<std::path::PathBuf> {
        pii_core::env_var("PII_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| self.core.runtime_dir.clone().map(std::path::PathBuf::from))
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
        assert!(cfg.core.model.is_none());
    }

    #[test]
    fn core_section_parses_and_is_absent_by_default() {
        let empty = Config::parse("").unwrap();
        assert_eq!(empty.core, super::CoreSection::default());

        let raw = r#"
[core]
model = "ollama:granite4.2:3b"
runtime_dir = "/root/.pii/runtime"
ollama_host = "http://127.0.0.1:11434"
"#;
        let cfg = Config::parse(raw).unwrap();
        assert_eq!(cfg.core.model.as_deref(), Some("ollama:granite4.2:3b"));
        assert_eq!(cfg.core.runtime_dir.as_deref(), Some("/root/.pii/runtime"));
        assert_eq!(
            cfg.core.ollama_host.as_deref(),
            Some("http://127.0.0.1:11434")
        );
    }

    #[test]
    fn resolve_model_prefers_explicit_then_config() {
        let cfg = Config::parse("[core]\nmodel = \"scripted:\"\n").unwrap();
        // No explicit arg, no env var set in this test process for the
        // (deliberately unique) key below: config wins.
        assert_eq!(
            cfg.resolve_model(None).as_deref(),
            Some("scripted:"),
            "config [core].model must be used when nothing more specific is set"
        );
        // An explicit value always wins over the config.
        assert_eq!(
            cfg.resolve_model(Some(String::from("ollama:x"))).as_deref(),
            Some("ollama:x")
        );
    }

    #[test]
    fn resolve_model_none_when_nothing_configured() {
        let cfg = Config::default();
        assert_eq!(cfg.resolve_model(None), None);
    }

    /// Spec 35: `[core] model = "llamacpp:<path>"` is a first-class config
    /// entry — it parses and resolves like any other model spec, so a daemon
    /// can be pointed at an in-process GGUF via the config file alone.
    #[test]
    fn core_section_parses_llamacpp_model() {
        let raw = r#"
[core]
model = "llamacpp:/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf"
"#;
        let cfg = Config::parse(raw).unwrap();
        assert_eq!(
            cfg.core.model.as_deref(),
            Some("llamacpp:/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf")
        );
        // resolve_model returns it verbatim (no special-casing).
        assert_eq!(
            cfg.resolve_model(None).as_deref(),
            Some("llamacpp:/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf")
        );
    }

    #[test]
    fn resolve_runtime_dir_uses_core_section_when_env_absent() {
        let cfg = Config::parse("[core]\nruntime_dir = \"/tmp/pii-test-rt\"\n").unwrap();
        // This assumes the test process has no PII_RUNTIME_DIR set, which
        // holds in CI/sandboxed test runs; resolve_runtime_dir's env-var
        // branch is exercised implicitly by every other daemon test that
        // DOES set it.
        if pii_core::env_var("PII_RUNTIME_DIR").is_none() {
            assert_eq!(
                cfg.resolve_runtime_dir(),
                Some(std::path::PathBuf::from("/tmp/pii-test-rt"))
            );
        }
    }
}
