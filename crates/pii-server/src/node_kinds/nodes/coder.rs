//! `coder` node kind (spec 207): the node spawns a coding agent (`pi` | `claude`) itself, and its verdict is the
//! run of its criteria, never the agent's own report.
//!
//! The argv and the refusals come from [`super::agent_worker`], the same planner the `worker` kind's pure half
//! uses. The spawn goes through [`pii_core::ProcCap::spawn_child_exact`], so the child's environment is exactly
//! the `env_pass` names copied from the daemon's environment. The node is an effect kind, so a run killed
//! mid-spawn is not dispatched a second time on resume.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::node::{GraphNode, NodeFactory};
use pii_observe::{Event, Level, Observer, names};

use super::agent_worker::{CriterionRun, Verdict, WorkerSpec, decide, plan};

/// Where the agent binaries live in production. A coder node may exec `pi` and `claude` from here, and nothing
/// else new.
pub const AGENT_BIN_DIR: &str = "/root/.local/bin";

/// The agent program paths under `bin_dir`: the only programs a `coder` node may exec (spec 207).
#[must_use]
pub fn agent_binaries(bin_dir: &std::path::Path) -> Vec<String> {
    ["pi", "claude"]
        .iter()
        .map(|agent| bin_dir.join(agent).display().to_string())
        .collect()
}

/// How long one criterion command may run before it is killed.
const CRITERION_TIMEOUT: Duration = Duration::from_mins(10);

/// How often a running agent is checked for cancellation and the run's deadline.
const POLL: Duration = Duration::from_millis(50);

/// Factory for `coder` nodes. The agent bin directory and the environment snapshot are fixed at construction:
/// production uses [`CoderFactory::new`]; tests inject their own.
pub struct CoderFactory {
    bin_dir: PathBuf,
    env: BTreeMap<String, String>,
    log: Arc<dyn Observer>,
}

impl CoderFactory {
    /// The production factory: agents under [`AGENT_BIN_DIR`], names copied from this process's environment.
    #[must_use]
    pub fn new(log: Arc<dyn Observer>) -> Self {
        Self::with_env(
            PathBuf::from(AGENT_BIN_DIR),
            std::env::vars().collect(),
            log,
        )
    }

    /// A factory over an explicit agent bin directory and environment.
    #[must_use]
    pub fn with_env(
        bin_dir: PathBuf,
        env: BTreeMap<String, String>,
        log: Arc<dyn Observer>,
    ) -> Self {
        Self { bin_dir, env, log }
    }
}

/// A built `coder` node. Its config is read at run time, so a bad config is a refusal, not a build error.
struct CoderNode {
    bin_dir: PathBuf,
    env: BTreeMap<String, String>,
    log: Arc<dyn Observer>,
    cfg: serde_json::Value,
}

fn text(cfg: &serde_json::Value, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn texts(cfg: &serde_json::Value, key: &str) -> Vec<String> {
    cfg.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The `err` port a refusal routes to: the scheduler never retries a refusal (spec 207 R6).
fn refused_port(agent: &str, model: &str, reason: &str) -> Outcome {
    Outcome::Next {
        port: PortName::from("err"),
        value: serde_json::json!({ "agent": agent, "model": model, "reason": reason }),
    }
}

impl CoderNode {
    /// Journals a refusal the node decided itself, then routes it to `err`.
    fn refuse(&self, agent: &str, model: &str, reason: &str) -> Outcome {
        self.log.event(
            &Event::new("coder", Level::Warn, names::WORKER_START_REFUSED)
                .field("agent", agent.to_owned())
                .field("model", model.to_owned())
                .field("reason", reason.to_owned()),
        );
        refused_port(agent, model, reason)
    }

    /// The daemon's values for exactly the names in `names`; a name the daemon lacks is simply absent.
    fn pass_env(&self, names: &[String]) -> BTreeMap<String, String> {
        names
            .iter()
            .filter_map(|n| self.env.get(n).map(|v| (n.clone(), v.clone())))
            .collect()
    }
}

impl GraphNode for CoderNode {
    fn run(&self, ctx: &Ctx, _input: &NodeInput) -> Result<Outcome, PiiError> {
        let agent = text(&self.cfg, "agent").unwrap_or_default();
        let model = text(&self.cfg, "model").unwrap_or_default();
        let (spec, brief) = match (WorkerSpec::from_cfg(&self.cfg), text(&self.cfg, "brief")) {
            (Ok(spec), Some(brief)) if !spec.spec.is_empty() => (spec, brief),
            (Err(e), _) => return Ok(self.refuse(&agent, &model, &format!("{e}"))),
            _ => return Ok(self.refuse(&agent, &model, "no spec or no brief")),
        };
        let env = self.pass_env(&texts(&self.cfg, "env_pass"));
        let dispatch = match plan(&spec, &brief, &env, "direct", self.log.as_ref()) {
            Ok(dispatch) => dispatch,
            Err(e) => return Ok(refused_port(&agent, &model, &format!("{e}"))),
        };
        let Some((program, rest)) = dispatch.argv.split_first() else {
            return Ok(self.refuse(&agent, &model, "empty agent argv"));
        };
        let mut argv = vec![self.bin_dir.join(program).display().to_string()];
        argv.extend(rest.iter().cloned());
        let workdir = PathBuf::from(&spec.workdir);
        let env_pairs: Vec<(String, String)> = env.into_iter().collect();

        let mut child = ctx.proc()?.spawn_child_exact(&argv, &workdir, &env_pairs)?;
        let pid = child.pid();
        self.log.event(
            &Event::new("coder", Level::Info, names::CODER_SPAWNED)
                .field("agent", agent)
                .field("model", model)
                .field("pid", i64::from(pid))
                .field("argv", argv.join(" ")),
        );
        // Nothing is written to the agent's stdin; its output is drained so a chatty agent cannot block.
        drop(child.stdin());
        if let Some(mut out) = child.stdout() {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut out, &mut std::io::sink());
            });
        }
        if let Some(mut err) = child.stderr() {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut err, &mut std::io::sink());
            });
        }
        let code = loop {
            if let Some(code) = child.wait_timeout(POLL)? {
                break code;
            }
            if let Err(e) = ctx.check() {
                let _ = child.kill();
                return Err(e);
            }
        };

        // The criteria decide. The first failing criterion stops the run and names itself in the verdict.
        let mut runs = Vec::new();
        for command in texts(&self.cfg, "criteria") {
            let shell = ["/bin/sh".to_owned(), "-c".to_owned(), command.clone()];
            let rc = ctx
                .proc()?
                .run(&shell, &workdir, CRITERION_TIMEOUT)?
                .exit_code;
            runs.push(CriterionRun { command, rc });
            if rc != 0 {
                break;
            }
        }
        match decide(code, "", &runs) {
            Verdict::Passed { criteria, .. } => Ok(Outcome::Next {
                port: PortName::from("ok"),
                value: serde_json::json!({
                    "pid": pid,
                    "rc": code,
                    "criteria": criteria
                        .iter()
                        .map(|c| serde_json::json!({ "command": c.command, "rc": c.rc }))
                        .collect::<Vec<_>>(),
                }),
            }),
            Verdict::Failed { reason, .. } => Ok(Outcome::Fail { error: reason }),
        }
    }
}

impl NodeFactory for CoderFactory {
    fn kind(&self) -> &'static str {
        "coder"
    }

    fn is_effect(&self) -> bool {
        true
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::SPAWN | CapKind::PROC
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn proc_argv0_grant(&self) -> Vec<String> {
        agent_binaries(&self.bin_dir)
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(CoderNode {
            bin_dir: self.bin_dir.clone(),
            env: self.env.clone(),
            log: Arc::clone(&self.log),
            cfg: cfg.clone(),
        }))
    }
}
