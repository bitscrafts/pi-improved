//! Coding-agent `worker` dispatch (spec 185): the agent (`pi` | `claude`) and its model are graph data.
//!
//! Pure planning and judging: [`plan`] turns node data into an argv or a refusal event, [`decide`] lets the
//! criteria — not the worker's own exit — decide the verdict, [`learn_fact`] turns an outcome into a memory
//! row, and [`escalate`] picks the next ladder rung from the previous attempt's evidence. The spawn itself goes
//! through a named launcher script that sets PATH and provider env inside it (HARD RULE 0d).

use std::collections::BTreeMap;

use pii_abi::PiiError;
use pii_observe::{Event, Level, Observer, names};

/// The API-key variable each `pi` provider reads (names only, never values). A `pi` dispatch that names a
/// provider is refused before spawn unless that provider's key is present, so the agent is never started
/// without the credential it needs. Keys come from the daemon environment via the graph's `env_pass`.
const PI_PROVIDER_KEYS: [(&str, &str); 2] = [
    ("ollama-cloud", "OLLAMA_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
];

/// The key variable a named `pi` provider needs, or `None` when the provider is not in the table.
#[must_use]
pub fn pi_provider_key(provider: &str) -> Option<&'static str> {
    PI_PROVIDER_KEYS
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, k)| *k)
}

/// The session file a `pi` dispatch writes: an explicit `session` as written, else one named for the spec and the run.
pub fn pi_session_path(spec: &WorkerSpec) -> String {
    spec.session.clone().unwrap_or_else(|| {
        let stem = std::path::Path::new(&spec.spec)
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("worker");
        let run = spec
            .run
            .as_ref()
            .map_or_else(String::new, |r| format!("-{r}"));
        format!("{}/.pii-sessions/{stem}{run}.jsonl", spec.workdir)
    })
}

/// Writes the session header `pi` needs before it will run.
///
/// An empty or missing session file is rejected by `pi` before any model is called, so the header is written
/// first. The shape is the one the installed `pi` emits (`type`, `version`, `id`, `timestamp`, `cwd`).
///
/// The timestamp comes from the caller's clock (`now_ms`), so the workspace's one sanctioned clock is the only
/// wall-clock read and a replay sees the same value. An existing file (a resumed session) is left untouched.
///
/// # Errors
///
/// Returns the I/O error when the parent directory cannot be created or the file cannot be written.
pub fn write_pi_session_header(path: &str, cwd: &str, now_ms: u64) -> std::io::Result<()> {
    let p = std::path::Path::new(path);
    if p.exists() {
        return Ok(());
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let id = uuid_v4();
    let ts = rfc3339_millis(now_ms);
    let header = format!(
        "{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"{ts}\",\"cwd\":{}}}\n",
        serde_json::Value::String(cwd.to_owned())
    );
    std::fs::write(p, header)
}

/// A v4 UUID from the OS random source, as text. Used only for the session id; no other meaning.
fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let mut h = String::with_capacity(32);
    for x in b {
        use std::fmt::Write as _;
        let _ = write!(h, "{x:02x}");
    }
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The millisecond RFC 3339 UTC form `pi` writes, for a given time. Pure formatting: it reads no clock.
fn rfc3339_millis(now_ms: u64) -> String {
    let secs = now_ms / 1000;
    let ms = u32::try_from(now_ms % 1000).unwrap_or(0);
    let (y, mo, da, hh, mi, ss) = civil_from_unix(secs);
    format!("{y:04}-{mo:02}-{da:02}T{hh:02}:{mi:02}:{ss:02}.{ms:03}Z")
}

/// Proleptic Gregorian date and time from a UNIX timestamp (days-from-civil, Howard Hinnant's algorithm).
fn civil_from_unix(secs: u64) -> (u64, u32, u32, u32, u32, u32) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        u32::try_from(m).unwrap_or(0),
        u32::try_from(d).unwrap_or(0),
        u32::try_from(rem / 3600).unwrap_or(0),
        u32::try_from((rem % 3600) / 60).unwrap_or(0),
        u32::try_from(rem % 60).unwrap_or(0),
    )
}

/// The provider and model an agent REPORTS it used, read from its `--mode json` event stream.
///
/// Observability only: this is what the run actually used, to compare with what was requested. Returns the
/// distinct `(provider, model)` pairs seen, in order of first appearance. An empty result means the output
/// named no route (a text run, or no output captured), which the caller must report as unknown, not as a match.
#[must_use]
pub fn reported_routes(output: &str) -> Vec<(String, String)> {
    let mut routes: Vec<(String, String)> = Vec::new();
    for line in output.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // `pi --mode json` names the route on its `session` header as `provider` + `modelId` (measured
        // 2026-10-21 against the installed pi). Any event that names `provider` + `model` also counts.
        // claude's stream-json names its model on the `system`/`init` line, with no provider field.
        if v.get("type").and_then(serde_json::Value::as_str) == Some("system")
            && v.get("subtype").and_then(serde_json::Value::as_str) == Some("init")
            && let Some(m) = v.get("model").and_then(serde_json::Value::as_str)
        {
            let pair = ("claude".to_owned(), m.to_owned());
            if !routes.contains(&pair) {
                routes.push(pair);
            }
            continue;
        }
        let provider = v.get("provider").and_then(serde_json::Value::as_str);
        let model = v
            .get("modelId")
            .or_else(|| v.get("model"))
            .and_then(serde_json::Value::as_str);
        if let (Some(p), Some(m)) = (provider, model) {
            let pair = (p.to_owned(), m.to_owned());
            if !routes.contains(&pair) {
                routes.push(pair);
            }
        }
    }
    routes
}

/// Which coding agent a worker drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    /// The `pi` coding agent.
    Pi,
    /// The `claude` CLI.
    Claude,
}

impl Agent {
    fn parse(s: &str) -> Result<Self, PiiError> {
        match s {
            "pi" => Ok(Self::Pi),
            "claude" => Ok(Self::Claude),
            other => Err(PiiError::Invalid(format!(
                "worker agent must be 'pi' or 'claude', got '{other}'"
            ))),
        }
    }
}

/// A worker node's configuration, read from the graph file.
#[derive(Debug, Clone)]
pub struct WorkerSpec {
    /// The agent to drive.
    pub agent: Agent,
    /// The model id handed to the agent.
    pub model: String,
    /// The provider the agent must use (`--provider`). Optional: when absent the agent keeps its own default.
    pub provider: Option<String>,
    /// Path of the spec the worker implements.
    pub spec: String,
    /// Working directory of the dispatch.
    pub workdir: String,
    /// Resume handle (claude session id), when one exists.
    pub sid: Option<String>,
    /// Session file (pi); defaults to one beside the spec's name under the workdir.
    pub session: Option<String>,
    /// The run whose session this dispatch writes; scopes the default session file. Set by the coder node.
    pub run: Option<String>,
    /// Spend cap in USD for one run (claude `--max-budget-usd`). `None` passes no flag. Always positive and finite.
    pub max_budget_usd: Option<f64>,
}

fn field(cfg: &serde_json::Value, key: &str) -> Result<String, PiiError> {
    cfg.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PiiError::Invalid(format!("worker node requires '{key}' string")))
}

fn opt(cfg: &serde_json::Value, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The optional `max_budget_usd`: absent is `None`; present must be a positive, finite number, so a typo can never
/// become "no cap" or a flag claude rejects after the dispatch has started.
fn budget(cfg: &serde_json::Value) -> Result<Option<f64>, PiiError> {
    let Some(v) = cfg.get("max_budget_usd") else {
        return Ok(None);
    };
    match v.as_f64() {
        Some(n) if n.is_finite() && n > 0.0 => Ok(Some(n)),
        _ => Err(PiiError::Invalid(
            "worker node 'max_budget_usd' must be a positive number".to_owned(),
        )),
    }
}

/// The graph's `sid`. A claude handle must be a session UUID v4 (spec 244 R5): a malformed one is refused here, before
/// any dispatch, so a bad handle never reaches `claude`. A pi node never reads `sid`, so it is passed through.
fn session_handle(agent: Agent, cfg: &serde_json::Value) -> Result<Option<String>, PiiError> {
    match opt(cfg, "sid") {
        Some(sid) if agent == Agent::Claude && !super::agent_claude::is_session_uuid(&sid) => {
            Err(PiiError::Invalid(
                "worker node 'sid' must be a lowercase UUID v4 session id".to_owned(),
            ))
        }
        sid => Ok(sid),
    }
}

/// The optional `run`: it names a file, so it must be `[A-Za-z0-9._-]+` and cannot leave `.pii-sessions/`.
fn run_handle(cfg: &serde_json::Value) -> Result<Option<String>, PiiError> {
    match opt(cfg, "run") {
        Some(r) if r.is_empty() || r == "." || r == ".." || !r.bytes().all(is_run_byte) => Err(
            PiiError::Invalid("worker node 'run' must match [A-Za-z0-9._-]+".to_owned()),
        ),
        run => Ok(run),
    }
}

fn is_run_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

impl WorkerSpec {
    /// Reads `agent`, `model`, `spec`, `workdir` (and optional `sid`, `session`, `run`, `max_budget_usd`) from node data.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a missing field, an unknown agent, a malformed claude `sid`, or a malformed `run`.
    pub fn from_cfg(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let agent = Agent::parse(&field(cfg, "agent")?)?;
        Ok(Self {
            agent,
            model: field(cfg, "model")?,
            provider: opt(cfg, "provider"),
            spec: field(cfg, "spec")?,
            workdir: field(cfg, "workdir")?,
            sid: session_handle(agent, cfg)?,
            session: opt(cfg, "session"),
            run: run_handle(cfg)?,
            max_budget_usd: budget(cfg)?,
        })
    }

    pub(super) fn agent_name(&self) -> &'static str {
        match self.agent {
            Agent::Pi => "pi",
            Agent::Claude => "claude",
        }
    }
}

/// A planned dispatch: the launcher script and the agent argv it execs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    /// The named script that sets PATH and provider env, then execs `argv`.
    pub launcher: String,
    /// The agent command line.
    pub argv: Vec<String>,
}

/// Plans the dispatch, or emits [`names::WORKER_START_REFUSED`] and errors without anything to spawn.
///
/// # Errors
/// [`PiiError::Unavailable`] when the agent's credentials are absent from `env`.
pub fn plan(
    spec: &WorkerSpec,
    brief: &str,
    env: &BTreeMap<String, String>,
    launcher: &str,
    log: &dyn Observer,
) -> Result<Dispatch, PiiError> {
    if spec.agent == Agent::Claude && !super::agent_claude::has_credential(env) {
        let reason = "no credentials in the environment";
        log.event(
            &Event::new("worker", Level::Warn, names::WORKER_START_REFUSED)
                .field("agent", spec.agent_name().to_owned())
                .field("model", spec.model.clone())
                .field("reason", reason.to_owned()),
        );
        return Err(PiiError::Unavailable(format!(
            "worker refused: agent {} model {}: {reason}",
            spec.agent_name(),
            spec.model
        )));
    }
    // A `pi` dispatch that names a provider must have that provider's key in its env. Refused before spawn,
    // through the same journal event as the claude check, so the agent never starts without its credential.
    if let (Agent::Pi, Some(provider)) = (spec.agent, spec.provider.as_deref()) {
        let key_ok =
            pi_provider_key(provider).is_some_and(|k| env.get(k).is_some_and(|v| !v.is_empty()));
        if !key_ok {
            let reason = format!("no credential for provider {provider} in the environment");
            log.event(
                &Event::new("worker", Level::Warn, names::WORKER_START_REFUSED)
                    .field("agent", spec.agent_name().to_owned())
                    .field("model", spec.model.clone())
                    .field("reason", reason.clone()),
            );
            return Err(PiiError::Unavailable(format!(
                "worker refused: agent {} model {}: {reason}",
                spec.agent_name(),
                spec.model
            )));
        }
    }
    // spec 244: a fresh claude run names its session before the spawn, so a crash after it still journals the handle.
    let claude_sid = spec.sid.as_deref().filter(|_| spec.agent == Agent::Claude);
    let new_session = (spec.agent == Agent::Claude && claude_sid.is_none()).then(uuid_v4);
    let s = |x: &str| x.to_owned();
    let argv = match spec.agent {
        Agent::Pi => {
            let mut v = vec![
                s("pi"),
                s("--session"),
                pi_session_path(spec),
                s("--model"),
                spec.model.clone(),
                // JSON events name the provider and model per message, so the run's real route can be read.
                s("--mode"),
                s("json"),
                // spec 362: a headless run has no approval surface, so pi's default `always-ask` denies every
                // mutating tool and the coder stops having written nothing (measured 2026-11-03: rc=3 after
                // 122 s, "approval required for `bash` but this session has no approval surface"). `write`
                // approves file edits and still refuses to run commands: the coder writes, the JUDGE runs the
                // criteria, and the daemon's capability set stays the outer bound.
                s("--approval-mode"),
                s("write"),
            ];
            if let Some(provider) = &spec.provider {
                v.extend([s("--provider"), provider.clone()]);
                // The key is NOT put on argv. pi resolves it at runtime from the env reference in the pii-owned
                // models.json (apiKey "$ENV:<NAME>"), with PI_CODING_AGENT_DIR pointing there. Measured 2026-10-22:
                // rc=0 via ollama-cloud, no --api-key. An argv key would show in /proc/<pid>/cmdline.
            }
            v.extend([s("-p"), brief.to_owned()]);
            v
        }
        Agent::Claude => super::agent_claude::argv(
            claude_sid,
            new_session.as_deref(),
            &spec.model,
            spec.max_budget_usd,
            brief,
        ),
    };
    Ok(Dispatch {
        launcher: launcher.to_owned(),
        argv,
    })
}

/// One run of one exit criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriterionRun {
    /// The criterion's command.
    pub command: String,
    /// Its exit code.
    pub rc: i32,
}

/// A worker node's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The worker exited zero and every criterion passed.
    Passed {
        /// The worker's own report.
        report: String,
        /// The criteria runs.
        criteria: Vec<CriterionRun>,
    },
    /// The node failed.
    Failed {
        /// Why, naming the failing criterion when one failed.
        reason: String,
        /// The first failing criterion's command.
        failing: Option<String>,
    },
}

/// The criteria decide: a clean worker exit with a failing (or absent) criterion is a FAILED node.
#[must_use]
pub fn decide(worker_rc: i32, report: &str, criteria: &[CriterionRun]) -> Verdict {
    if worker_rc != 0 {
        return Verdict::Failed {
            reason: format!("worker exited {worker_rc}"),
            failing: None,
        };
    }
    if criteria.is_empty() {
        return Verdict::Failed {
            reason: "no criteria were run".into(),
            failing: None,
        };
    }
    if let Some(bad) = criteria.iter().find(|c| c.rc != 0) {
        return Verdict::Failed {
            reason: format!("criterion failed (rc {}): {}", bad.rc, bad.command),
            failing: Some(bad.command.clone()),
        };
    }
    Verdict::Passed {
        report: report.to_owned(),
        criteria: criteria.to_vec(),
    }
}

/// The memory row for one outcome: spec, agent, model, verdict and failing criterion.
#[must_use]
pub fn learn_fact(
    spec: &str,
    agent: &str,
    model: &str,
    verdict: &Verdict,
    now_ms: u64,
) -> pii_store::mem::PutRows {
    let (word, failing, importance) = match verdict {
        Verdict::Passed { .. } => ("passed", String::new(), 0.4),
        Verdict::Failed { reason, failing } => (
            "failed",
            format!(
                " failing criterion: {}",
                failing.as_deref().unwrap_or(reason)
            ),
            0.8,
        ),
    };
    let text = format!("worker outcome: spec {spec} agent {agent} model {model} {word}.{failing}");
    pii_memory::doc::write_fact(
        &text,
        vec![agent.to_owned(), model.to_owned(), format!("spec-{spec}")],
        importance,
        now_ms,
    )
}

/// What the previous attempt left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptEvidence {
    /// The failing criterion; `None` means the attempt passed or left no evidence.
    pub failing_criterion: Option<String>,
}

/// One rung of a heal ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rung {
    /// `pi` or `claude`.
    pub agent: String,
    /// The model id.
    pub model: String,
}

/// An ordered list of rungs, cheapest first.
#[derive(Debug, Clone)]
pub struct Ladder(pub Vec<Rung>);

impl Ladder {
    /// Reads `[{agent, model}, …]`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on an empty ladder, a malformed rung or an unknown agent.
    pub fn from_cfg(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        let items = cfg
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| PiiError::Invalid("ladder must be a non-empty array".into()))?;
        let mut rungs = Vec::new();
        for it in items {
            let agent = field(it, "agent")?;
            Agent::parse(&agent)?;
            rungs.push(Rung {
                agent,
                model: field(it, "model")?,
            });
        }
        Ok(Self(rungs))
    }
}

/// The next rung given the attempts so far: rung 0 with no history; otherwise the next rung only when every
/// previous attempt failed WITH evidence. A passing or evidence-free attempt ends the ladder.
#[must_use]
pub fn escalate<'a>(ladder: &'a Ladder, history: &[AttemptEvidence]) -> Option<&'a Rung> {
    if history.iter().any(|a| a.failing_criterion.is_none()) {
        return None;
    }
    ladder.0.get(history.len())
}

#[cfg(test)]
mod civil_time_tests {
    use super::civil_from_unix;

    #[test]
    fn the_unix_epoch_is_1970_01_01() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn a_leap_day_is_handled() {
        // 2000-02-29 12:34:56 UTC = 951_827_696 (a 400-year leap year).
        assert_eq!(civil_from_unix(951_827_696), (2000, 2, 29, 12, 34, 56));
    }

    #[test]
    fn the_day_after_a_year_end_rolls_over() {
        // 2026-12-31 23:59:59 UTC = 1_798_761_599; one second later is 2027-01-01.
        assert_eq!(civil_from_unix(1_798_761_599), (2026, 12, 31, 23, 59, 59));
        assert_eq!(civil_from_unix(1_798_761_600), (2027, 1, 1, 0, 0, 0));
    }
}
