//! Daemon config (F5.2): one TOML file, `deny_unknown_fields` throughout —
//! a typo'd key must be a load error, not a silently-ignored no-op.
//!
//! **Complete env surface for this phase** (N10, every read goes through
//! [`pii_core::env_var`]): `PII_CONFIG`, `PII_RUNTIME_DIR`, `PII_MODEL`,
//! `PII_MEMORY_PATH`, `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID`,
//! `PII_DISCORD_API_BASE`, `OPENAI_BASE_URL`, `OPENAI_API_KEY`.
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
    /// Default model spec (e.g. `"llamacpp-server:granite-4.2-3b"`, `"scripted:"`).
    /// Overridden by `--model` (oneshot) or `$PII_MODEL`, in that order.
    pub model: Option<String>,
    /// Default runtime dir (holds `control.sock`, `graphs/`). Overridden
    /// by `$PII_RUNTIME_DIR`.
    pub runtime_dir: Option<String>,
    /// Directory for the durable stores (channel cursor, timer wheel) (spec 337).
    /// Absent = `<state_dir>/durable`; never the runtime dir (a tmpfs on the host).
    pub durable_dir: Option<String>,
    /// Max runs retained per session in the in-memory `session_runs` index
    /// (D3/D6, 026 report 5.0.4): oldest run ids are evicted (LRU) once a
    /// session's run count exceeds this. `None` (absent key) falls back to
    /// [`crate::DEFAULT_SESSION_RUNS_CAP`] — see [`Self::session_runs_cap`].
    pub session_runs_cap: Option<usize>,
    /// Root directory for run trajectories (spec 103); each run appends to
    /// `<root>/trajectories/<run_id>.jsonl`. Overridden by
    /// `$PII_TRAJECTORY_DIR`. Absent = trajectories OFF.
    pub trajectory_dir: Option<String>,
    /// Aggregate byte bound on the trajectory directory (spec 115); oldest
    /// finished runs are pruned past it. Absent = 512 MiB.
    pub trajectory_max_bytes: Option<u64>,
    /// Aggregate file-count bound on the trajectory directory (spec 115).
    /// Absent = 50 000.
    pub trajectory_max_files: Option<usize>,
    /// Seconds between unattended extraction + verification passes (spec 139).
    /// Needs a trajectory root. Absent = [`DEFAULT_LEARN_INTERVAL_SECS`] (daily);
    /// `0` = off. The pass proposes and verifies only: promotion stays a human
    /// act, and the loop's pre-registered decision rule is in
    /// `_docs/02-guides/learning-loop.md` (14 days, `pii metrics`).
    pub learn_interval_secs: Option<u64>,
    /// Daemon log file (spec 104): contract events as JSON lines. Overridden
    /// by `--log-file`. Absent = no log file.
    pub log_file: Option<String>,
    /// Rotate the log file once it would exceed this many bytes. Overridden
    /// by `--log-rotate-bytes`. Absent = [`DEFAULT_LOG_ROTATE_BYTES`].
    pub log_rotate_bytes: Option<u64>,
    /// Rotated log files kept besides the live one. Overridden by
    /// `--log-keep-files`. Absent = [`DEFAULT_LOG_KEEP_FILES`].
    pub log_keep_files: Option<usize>,
    /// `"on"` (default) or `"off"` (spec 116, measurement only): `off` detaches
    /// the daemon's observer fan-out. Overridden by `--observer`.
    pub observer: Option<String>,
    /// `"on"` (default) or `"off"` (spec 116, measurement only): `off` stops
    /// the audit feed. Overridden by `--audit`.
    pub audit: Option<String>,
}

/// `[llama]` (spec 129): where the `llamacpp-server:<model>` kind dials.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct LlamaSection {
    /// Base URL of the llama.cpp server's OpenAI-compatible API, e.g.
    /// `"http://10.10.10.217:8082/v1"`. Absent = loopback
    /// (`http://127.0.0.1:8080/v1`). The local provider's `NetService` is
    /// scoped to exactly this host.
    pub host: Option<String>,
}

/// Default size at which the log file rotates (10 MiB).
pub const DEFAULT_LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;
/// Default seconds between unattended learn passes: once a day (spec 139).
pub const DEFAULT_LEARN_INTERVAL_SECS: u64 = 24 * 60 * 60;
/// Default number of rotated log files kept.
pub const DEFAULT_LOG_KEEP_FILES: usize = 5;

impl CoreSection {
    /// The effective trajectory retention policy (spec 115): configured
    /// bounds, else the defaults. A `0` is read as "unset" (the default),
    /// never as "unbounded": the directory is always bounded.
    #[must_use]
    pub fn trajectory_retention(&self) -> pii_observe::Retention {
        let d = pii_core::DEFAULT_TRAJECTORY_RETENTION;
        pii_observe::Retention {
            max_bytes: self.trajectory_max_bytes.filter(|n| *n > 0).or(d.max_bytes),
            max_files: self.trajectory_max_files.filter(|n| *n > 0).or(d.max_files),
        }
    }

    /// The unattended learn period (spec 139); `None` when switched off (`0`).
    #[must_use]
    pub fn learn_interval(&self) -> Option<std::time::Duration> {
        let n = self
            .learn_interval_secs
            .unwrap_or(DEFAULT_LEARN_INTERVAL_SECS);
        (n > 0).then(|| std::time::Duration::from_secs(n))
    }

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

/// `[[model_tier]]` (spec 114): a named model class a graph node can ask for.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelTierSection {
    /// Tier name, referenced by a node's `tier` key. Unique.
    pub name: String,
    /// Provider spec (`scripted:<path>`, `llamacpp-server:<m>`, `openai:<m>`, ...).
    pub model: String,
    /// Tools a run on this tier may call. Absent = unrestricted. When set it
    /// is enforced by the capability layer, not by the prompt.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Upper bound on `max_tokens` per model request on this tier.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// `[[channel]]` (F5.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSection {
    /// Channel name.
    pub name: String,
    /// Channel kind (`discord`, `cli`, `telegram`, `matrix`, `webhook`).
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
    /// `telegram`/`matrix`: env var holding the token (default
    /// `PII_TELEGRAM_TOKEN` / `PII_MATRIX_TOKEN`). The token is never a config value.
    #[serde(default)]
    pub token_env: Option<String>,
    /// `telegram`/`matrix`: a 0600 file holding the token (used when the env var is unset).
    #[serde(default)]
    pub token_file: Option<String>,
    /// `telegram`: Bot API base; `matrix`: the homeserver base URL (required).
    #[serde(default)]
    pub base_url: Option<String>,
    /// `matrix` only: the bot's own user id (its events are skipped).
    #[serde(default)]
    pub user_id: Option<String>,
    /// `webhook` only: listen address, default `127.0.0.1:8787`. A
    /// non-loopback address makes the shared secret mandatory.
    #[serde(default)]
    pub bind: Option<String>,
    /// `webhook` only: URL that `send` POSTs `{to, text}` to (egress-allowlisted).
    #[serde(default)]
    pub url: Option<String>,
    /// `webhook` only: env var holding the shared secret (`X-Pii-Secret`).
    #[serde(default)]
    pub secret_env: Option<String>,
    /// `webhook` only: a 0600 file holding the shared secret.
    #[serde(default)]
    pub secret_file: Option<String>,
    /// Spec 137: `host:port` targets the operator sanctions for this channel's egress, for a host the
    /// channel takes from its environment (`discord` with `PII_DISCORD_API_BASE`).
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// Discord answers `GET /channels/{id}/messages` at roughly five requests per five seconds per route, so
/// one request per second is the ceiling and anything faster is refused with `429 retry_after`.
///
/// Measured 2026-11-02 on production: `poll_interval_ms` defaulted to 500 and the daemon collected **173**
/// `channel.poll.failed 429` in a few hours, never reading a message. 1500 ms leaves the bucket room to
/// refill between reads.
const fn default_poll_ms() -> u64 {
    1_500
}

const fn default_mqtt_poll_ms() -> u64 {
    100
}

/// `[[mqtt]]` (spec 117): one broker connection.
///
/// It subscribes to `topics` and starts `graph` per message (input
/// `{topic, payload, text, channel}`), and is the target of `mqtt` publish
/// nodes naming it as `broker`. Credentials are
/// never config values: `*_env` names an environment variable, `*_file` a 0600
/// file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttSection {
    /// Connection name (also its health subsystem, `channel.<name>`).
    pub name: String,
    /// `mqtt://host[:port]`; `mqtts://` is refused (no TLS in this build).
    pub url: String,
    /// Graph run per inbound message.
    pub graph: String,
    /// Topic filters to subscribe to; empty = publish-only.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Subscription `QoS` (0..=2).
    #[serde(default)]
    pub qos: u8,
    /// MQTT client id; default is unique per daemon instance.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Env var holding the broker user name.
    #[serde(default)]
    pub username_env: Option<String>,
    /// 0600 file holding the broker user name.
    #[serde(default)]
    pub username_file: Option<String>,
    /// Env var holding the broker password.
    #[serde(default)]
    pub password_env: Option<String>,
    /// 0600 file holding the broker password.
    #[serde(default)]
    pub password_file: Option<String>,
    /// Accept sending credentials in clear text to a non-loopback broker.
    #[serde(default)]
    pub allow_plaintext_credentials: bool,
    /// First reconnect delay in ms (doubles up to `reconnect_max_ms`).
    #[serde(default = "default_reconnect_min")]
    pub reconnect_min_ms: u64,
    /// Reconnect delay ceiling in ms.
    #[serde(default = "default_reconnect_max")]
    pub reconnect_max_ms: u64,
    /// Bound on the inbound queue (messages beyond it are dropped and counted).
    #[serde(default = "default_queue_cap")]
    pub queue_cap: usize,
    /// How often the router drains the inbound queue, in ms.
    #[serde(default = "default_mqtt_poll_ms")]
    pub poll_interval_ms: u64,
}

const fn default_reconnect_min() -> u64 {
    pii_channels::mqtt::DEFAULT_RECONNECT_MIN_MS
}

const fn default_reconnect_max() -> u64 {
    pii_channels::mqtt::DEFAULT_RECONNECT_MAX_MS
}

const fn default_queue_cap() -> usize {
    pii_channels::mqtt::DEFAULT_QUEUE_CAP
}

/// `[hal]` (spec 121): the hardware backend and the pins/buses a graph may
/// claim. Absent = the HAL is off, and a `hal` node fails the run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HalSection {
    /// `sim` (in-memory, deterministic) or `gpiod` (real; needs the `hal-gpiod` build feature).
    pub backend: String,
    /// GPIO lines a graph may claim.
    #[serde(default)]
    pub pins: Vec<u32>,
    /// I2C addresses a graph may claim, as `bus:addr` (`1:0x48`).
    #[serde(default)]
    pub i2c: Vec<String>,
    /// SPI buses a graph may claim.
    #[serde(default)]
    pub spi: Vec<u8>,
    /// `gpiod` only: the GPIO character device (default `/dev/gpiochip0`).
    #[serde(default)]
    pub chip: Option<String>,
}

/// `[wasm]` (spec 122): where plugin modules may be loaded from and the limits
/// every call runs under. Absent = the plugin host is off, and a `wasm` node
/// fails the run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmSection {
    /// The only directory modules may be loaded from.
    pub dir: String,
    /// Wall-clock budget per call, in milliseconds (a node may only lower it).
    #[serde(default = "default_wasm_timeout_ms")]
    pub timeout_ms: u64,
    /// Memory ceiling per instance, in MiB.
    #[serde(default = "default_wasm_memory_mib")]
    pub max_memory_mib: usize,
    /// Fuel a node gets with `fuel = true`; also the most it may ask for.
    #[serde(default = "default_wasm_fuel")]
    pub fuel: u64,
}

const fn default_wasm_timeout_ms() -> u64 {
    1000
}

const fn default_wasm_memory_mib() -> usize {
    16
}

const fn default_wasm_fuel() -> u64 {
    pii_wasm::DEFAULT_FUEL
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

/// `[memory_graph]` (spec 126): the bounds on the entity and relationship store.
/// Absent = the defaults, which are [`pii_memory::entity::GraphConfig::default`].
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct MemoryGraphSection {
    /// Most entities (default 10000); the next new one is refused as `entity_cap`.
    pub max_entities: Option<usize>,
    /// Most relationships (default 50000); the next new one is refused as `relationship_cap`.
    pub max_relationships: Option<usize>,
    /// Most provenance sources per record (default 32); one more is refused as `provenance_cap`.
    pub max_provenance: Option<usize>,
    /// Most attributes per record (default 32).
    pub max_attributes: Option<usize>,
    /// Deepest neighbour walk (default 3).
    pub max_depth: Option<u32>,
    /// Neighbour depth when a call names none (default 1).
    pub default_depth: Option<u32>,
    /// Most results a query may return (default 200); more is refused as `result_cap`.
    pub max_results: Option<usize>,
    /// Seconds a read keeps a record `hot` (default 86400; spec 127).
    pub hot_secs: Option<u64>,
    /// Seconds a read keeps a record at least `warm`; older is `cold` (default 604800).
    pub warm_secs: Option<u64>,
    /// Reads that keep a record `hot` through the warm window (default 10).
    pub hot_count: Option<u64>,
    /// Byte budget for the graph file; the write that would pass it is refused as `bytes_cap` (default 16 MiB).
    pub max_bytes: Option<u64>,
}

impl MemoryGraphSection {
    /// The effective bounds: the defaults with this section's keys applied.
    #[must_use]
    pub fn graph_config(&self) -> pii_memory::entity::GraphConfig {
        let d = pii_memory::entity::GraphConfig::default();
        pii_memory::entity::GraphConfig {
            max_entities: self.max_entities.unwrap_or(d.max_entities),
            max_relationships: self.max_relationships.unwrap_or(d.max_relationships),
            max_provenance: self.max_provenance.unwrap_or(d.max_provenance),
            max_attributes: self.max_attributes.unwrap_or(d.max_attributes),
            max_depth: self.max_depth.unwrap_or(d.max_depth),
            default_depth: self.default_depth.unwrap_or(d.default_depth),
            max_results: self.max_results.unwrap_or(d.max_results),
            hot_secs: self.hot_secs.unwrap_or(d.hot_secs),
            warm_secs: self.warm_secs.unwrap_or(d.warm_secs),
            hot_count: self.hot_count.unwrap_or(d.hot_count),
            max_bytes: self.max_bytes.unwrap_or(d.max_bytes),
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
    /// Spec 215: the capability names no graph or node may request beyond; empty = no ceiling.
    pub grant_ceiling: Vec<String>,
    /// `[core]`.
    pub core: CoreSection,
    /// `[llama]` (spec 129).
    pub llama: LlamaSection,
    /// `[net]`.
    pub net: NetSection,
    /// `[[worker]]`.
    pub worker: Vec<WorkerSection>,
    /// `[[channel]]`.
    pub channel: Vec<ChannelSection>,
    /// `[[mqtt]]` (spec 117).
    pub mqtt: Vec<MqttSection>,
    /// `[hal]` (spec 121); `None` = the HAL is off.
    pub hal: Option<HalSection>,
    /// `[wasm]` (spec 122); `None` = the plugin host is off.
    pub wasm: Option<WasmSection>,
    /// `[memory]`.
    pub memory: MemorySection,
    /// `[memory_graph]` (spec 126).
    pub memory_graph: MemoryGraphSection,
    /// `[[cron]]`.
    pub cron: Vec<CronSection>,
    /// `[evolve]` (spec 28).
    #[serde(default)]
    pub evolve: EvolveSection,
    /// `[[model_tier]]` (spec 114).
    pub model_tier: Vec<ModelTierSection>,
}

impl Config {
    /// The `grant_ceiling` names as one capability set; `None` when the config names none (spec 215).
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for an unknown capability name.
    pub fn grant_ceiling(&self) -> Result<Option<pii_core::CapKind>, PiiError> {
        if self.grant_ceiling.is_empty() {
            return Ok(None);
        }
        let mut kinds = pii_core::CapKind::empty();
        for name in &self.grant_ceiling {
            kinds |= pii_core::CapKind::parse_kind(name)?;
        }
        Ok(Some(kinds))
    }

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
        // The ollama provider was retired (spec 129): its host key is refused
        // by name rather than as a generic unknown field.
        let probe: Option<toml::Value> = toml::from_str(raw).ok();
        if probe
            .as_ref()
            .and_then(|v| v.get("core"))
            .and_then(|c| c.get("ollama_host"))
            .is_some()
        {
            return Err(PiiError::Invalid(String::from(
                "config: [core].ollama_host is retired (spec 129) — set `[llama] host` for the llama.cpp server instead",
            )));
        }
        let cfg: Self =
            toml::from_str(raw).map_err(|e| PiiError::Invalid(format!("config: {e}")))?;
        cfg.validate_tiers()?;
        Ok(cfg)
    }

    /// A tier needs a non-empty, unique name and a model (spec 114 R1).
    fn validate_tiers(&self) -> Result<(), PiiError> {
        let mut seen = std::collections::BTreeSet::new();
        for t in &self.model_tier {
            if t.name.is_empty() || t.model.is_empty() {
                return Err(PiiError::Invalid(String::from(
                    "config: [[model_tier]] needs a non-empty name and model",
                )));
            }
            if !seen.insert(t.name.as_str()) {
                return Err(PiiError::Invalid(format!(
                    "config: duplicate [[model_tier]] name '{}'",
                    t.name
                )));
            }
        }
        Ok(())
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
        crate::tiers::precedence(
            None,
            None,
            explicit,
            pii_core::env_var("PII_MODEL"),
            self.core.model.clone(),
        )
        .map(|(_, m)| m)
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

    /// The durable-store dir (spec 337 R2): `[core].durable_dir` when set, else
    /// `<state_dir>/durable`. Never defaulted to the runtime dir.
    #[must_use]
    pub fn resolve_durable_dir(&self, state_dir: &std::path::Path) -> std::path::PathBuf {
        self.core
            .durable_dir
            .clone()
            .map_or_else(|| state_dir.join("durable"), std::path::PathBuf::from)
    }
}

/// A [`pii_net::grant::GrantStore`] giving every principal the same grant.
///
/// Scoped by the config's `grants` host patterns (F5.2): this phase needs
/// no per-principal partitioning — `pii-net`'s own suite covers isolation
/// at the policy-engine level.
pub struct ConfigGrants {
    patterns: Vec<String>,
    connect: pii_net::grant::HostScope,
    allow_private: bool,
    /// Spec 132: hosts a channel declared (`Channel::egress_hosts`), granted to that channel's
    /// principal alone, on top of `[net] grants`.
    declared: parking_lot::Mutex<BTreeMap<String, Vec<String>>>,
}

impl ConfigGrants {
    /// Builds from the `[net]` section.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for a malformed host pattern.
    pub fn from_section(net: &NetSection) -> Result<Self, PiiError> {
        Ok(Self {
            patterns: net.grants.clone(),
            connect: pii_net::grant::HostScope::parse_all(&net.grants)?,
            allow_private: net.allow_private,
            declared: parking_lot::Mutex::default(),
        })
    }

    /// True when `[net] grants` covers `host_port` (`host:port`).
    #[must_use]
    pub fn net_covers(&self, host_port: &str) -> bool {
        let Some((host, port)) = host_port.rsplit_once(':') else {
            return false;
        };
        port.parse::<u16>()
            .is_ok_and(|p| self.connect.permits(host, p))
    }

    /// Grants `hosts` (`host:port`) to `principal` only. An empty list grants nothing.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for a malformed host pattern.
    pub fn allow_declared(&self, principal: &str, hosts: &[String]) -> Result<(), PiiError> {
        if hosts.is_empty() {
            return Ok(());
        }
        pii_net::grant::HostScope::parse_all(hosts)?;
        self.declared
            .lock()
            .entry(principal.to_owned())
            .or_default()
            .extend(hosts.iter().cloned());
        Ok(())
    }

    fn one_grant(&self) -> pii_net::grant::NetGrant {
        self.grant_with(&self.connect)
    }

    fn grant_with(&self, connect: &pii_net::grant::HostScope) -> pii_net::grant::NetGrant {
        pii_net::grant::NetGrant {
            connect: connect.clone(),
            allow_private: self.allow_private,
            max_conns: 64,
            max_bytes_out: 16 * 1024 * 1024,
            max_bytes_in: 16 * 1024 * 1024,
            expires_at_ms: None,
        }
    }
}

impl pii_net::grant::GrantStore for ConfigGrants {
    fn grant(&self, principal: &str, _key: &str) -> Option<pii_net::grant::NetGrant> {
        let declared = self.declared.lock().get(principal).cloned();
        let Some(extra) = declared else {
            return Some(self.one_grant());
        };
        let all: Vec<String> = self.patterns.iter().cloned().chain(extra).collect();
        // Every pattern was validated when it was added, so this parse cannot fail; if it ever
        // did, the config-only grant is the safe answer.
        let scope =
            pii_net::grant::HostScope::parse_all(&all).unwrap_or_else(|_| self.connect.clone());
        Some(self.grant_with(&scope))
    }

    fn all(&self) -> Vec<(String, String, pii_net::grant::NetGrant)> {
        vec![(String::from("*"), String::from("*"), self.one_grant())]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Config;

    /// Spec 115 R2: the retention keys parse, default to a real bound, and a
    /// zero never means "unbounded".
    #[test]
    fn mqtt_section_parses_and_refuses_a_credential_value() {
        let c = Config::parse(
            "[[mqtt]]\nname = \"p\"\nurl = \"mqtt://127.0.0.1\"\ngraph = \"g\"\n\
             topics = [\"a/#\"]\npassword_env = \"X\"\n",
        )
        .unwrap();
        let m = c.mqtt.first().unwrap();
        assert_eq!((m.qos, m.queue_cap, m.reconnect_max_ms), (0, 1024, 30_000));
        assert!(!m.allow_plaintext_credentials);
        assert!(
            Config::parse("[[mqtt]]\nname=\"p\"\nurl=\"u\"\ngraph=\"g\"\npassword=\"x\"\n")
                .is_err(),
            "a password value is not a config key"
        );
    }

    #[test]
    fn trajectory_retention_keys_parse_and_zero_is_not_unbounded() {
        let c = Config::parse("[core]\ntrajectory_max_bytes = 1000\ntrajectory_max_files = 7\n")
            .unwrap();
        let r = c.core.trajectory_retention();
        assert_eq!((r.max_bytes, r.max_files), (Some(1000), Some(7)));
        let d = Config::parse("").unwrap().core.trajectory_retention();
        assert_eq!(d, pii_core::DEFAULT_TRAJECTORY_RETENTION);
        let z =
            Config::parse("[core]\ntrajectory_max_bytes = 0\ntrajectory_max_files = 0\n").unwrap();
        assert_eq!(
            z.core.trajectory_retention(),
            pii_core::DEFAULT_TRAJECTORY_RETENTION
        );
    }

    #[test]
    fn learn_interval_defaults_to_daily_zero_is_off_and_typos_fail() {
        let day = std::time::Duration::from_secs(super::DEFAULT_LEARN_INTERVAL_SECS);
        assert_eq!(Config::parse("").unwrap().core.learn_interval(), Some(day));
        let off = Config::parse("[core]\nlearn_interval_secs = 0\n").unwrap();
        assert_eq!(off.core.learn_interval(), None);
        let hourly = Config::parse("[core]\nlearn_interval_secs = 3600\n").unwrap();
        assert_eq!(
            hourly.core.learn_interval(),
            Some(std::time::Duration::from_hours(1))
        );
        assert!(Config::parse("[core]\nlearn_intervals = 1\n").is_err());
    }

    #[test]
    fn trajectory_dir_parses_and_defaults_to_off() {
        let on = Config::parse("[core]\ntrajectory_dir = \"/var/pii\"\n").unwrap();
        assert_eq!(on.core.trajectory_dir.as_deref(), Some("/var/pii"));
        assert_eq!(Config::parse("").unwrap().core.trajectory_dir, None);
    }

    #[test]
    fn the_checked_in_production_sample_parses() {
        let raw = include_str!("../../../../examples/pii.production.toml");
        let cfg = Config::parse(raw).unwrap();
        assert!(cfg.core.model.is_some());
        assert!(cfg.core.log_file.is_some());
        assert_eq!(cfg.core.log_keep_files, Some(5));
    }

    #[test]
    fn log_keys_parse_and_default_to_off() {
        let on = Config::parse(
            "[core]\nlog_file = \"/var/log/pii.log\"\nlog_rotate_bytes = 4096\nlog_keep_files = 2\n",
        )
        .unwrap();
        assert_eq!(on.core.log_file.as_deref(), Some("/var/log/pii.log"));
        assert_eq!(on.core.log_rotate_bytes, Some(4096));
        assert_eq!(on.core.log_keep_files, Some(2));
        let off = Config::parse("").unwrap();
        assert_eq!(off.core.log_file, None);
    }

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
model = "llamacpp-server:granite-4.2-3b"
runtime_dir = "/root/.pii/runtime"

[llama]
host = "http://10.10.10.217:8082/v1"
"#;
        let cfg = Config::parse(raw).unwrap();
        assert_eq!(
            cfg.core.model.as_deref(),
            Some("llamacpp-server:granite-4.2-3b")
        );
        assert_eq!(cfg.core.runtime_dir.as_deref(), Some("/root/.pii/runtime"));
        assert_eq!(
            cfg.llama.host.as_deref(),
            Some("http://10.10.10.217:8082/v1")
        );
        assert_eq!(
            empty.llama.host, None,
            "default is loopback, applied downstream"
        );
    }

    /// Spec 129: the retired key is refused loudly, naming `[llama] host`.
    #[test]
    fn a_config_with_the_retired_ollama_host_key_is_refused() {
        let err = Config::parse("[core]\nollama_host = \"http://127.0.0.1:11434\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("[llama] host"), "{err}");
        assert!(err.contains("ollama_host"), "{err}");
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
            cfg.resolve_model(Some(String::from("scripted:x")))
                .as_deref(),
            Some("scripted:x")
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
model = "llamacpp:/models/granite/granite-4.2-3b-Q4_K_M.gguf"
"#;
        let cfg = Config::parse(raw).unwrap();
        assert_eq!(
            cfg.core.model.as_deref(),
            Some("llamacpp:/models/granite/granite-4.2-3b-Q4_K_M.gguf")
        );
        // resolve_model returns it verbatim (no special-casing).
        assert_eq!(
            cfg.resolve_model(None).as_deref(),
            Some("llamacpp:/models/granite/granite-4.2-3b-Q4_K_M.gguf")
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
