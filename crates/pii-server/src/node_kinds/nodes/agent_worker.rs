//! Coding-agent `worker` dispatch (spec 185): the agent (`pi` | `claude`) and its model are graph data.
//!
//! Pure planning and judging: [`plan`] turns node data into an argv or a refusal event, [`decide`] lets the
//! criteria — not the worker's own exit — decide the verdict, [`learn_fact`] turns an outcome into a memory
//! row, and [`escalate`] picks the next ladder rung from the previous attempt's evidence. The spawn itself goes
//! through a named launcher script that sets PATH and provider env inside it (HARD RULE 0d).

use std::collections::BTreeMap;

use pii_abi::PiiError;
use pii_observe::{Event, Level, Observer, names};

/// Environment variables, any one of which authorises a `claude` dispatch.
const CLAUDE_CREDENTIALS: [&str; 2] = ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];

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
    /// Path of the spec the worker implements.
    pub spec: String,
    /// Working directory of the dispatch.
    pub workdir: String,
    /// Resume handle (claude session id), when one exists.
    pub sid: Option<String>,
    /// Session file (pi); defaults to one beside the spec's name under the workdir.
    pub session: Option<String>,
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

impl WorkerSpec {
    /// Reads `agent`, `model`, `spec`, `workdir` (and optional `sid`, `session`) from node data.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on a missing field or an unknown agent.
    pub fn from_cfg(cfg: &serde_json::Value) -> Result<Self, PiiError> {
        Ok(Self {
            agent: Agent::parse(&field(cfg, "agent")?)?,
            model: field(cfg, "model")?,
            spec: field(cfg, "spec")?,
            workdir: field(cfg, "workdir")?,
            sid: opt(cfg, "sid"),
            session: opt(cfg, "session"),
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
    if spec.agent == Agent::Claude
        && !CLAUDE_CREDENTIALS
            .iter()
            .any(|k| env.get(*k).is_some_and(|v| !v.is_empty()))
    {
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
    let s = |x: &str| x.to_owned();
    let argv = match spec.agent {
        Agent::Pi => {
            let file = spec.session.clone().unwrap_or_else(|| {
                let stem = std::path::Path::new(&spec.spec)
                    .file_stem()
                    .and_then(|x| x.to_str())
                    .unwrap_or("worker");
                format!("{}/.pii-sessions/{stem}.jsonl", spec.workdir)
            });
            vec![
                s("pi"),
                s("--session"),
                file,
                s("--model"),
                spec.model.clone(),
                s("-p"),
                brief.to_owned(),
            ]
        }
        Agent::Claude => {
            let mut v = vec![s("claude")];
            if let Some(sid) = &spec.sid {
                v.extend([s("--resume"), sid.clone()]);
            }
            v.extend([s("--model"), spec.model.clone(), s("-p"), brief.to_owned()]);
            v
        }
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
