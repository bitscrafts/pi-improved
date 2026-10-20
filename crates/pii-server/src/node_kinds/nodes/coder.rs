//! `coder` node kind (spec 207): the node spawns a coding agent (`pi` | `claude`) itself, and its verdict is the
//! run of its criteria, never the agent's own report.
//!
//! The argv and the refusals come from [`super::agent_worker`], the same planner the `worker` kind's pure half
//! uses. The spawn goes through [`pii_core::ProcCap::spawn_child_exact`], so the child's environment is exactly
//! the `env_pass` names copied from the daemon's environment. The node is an effect kind, so a run killed
//! mid-spawn is not dispatched a second time on resume.
//!
//! Spec 209: a node that names a `container_image` runs its agent inside that image through podman
//! ([`super::coder_container`]), so the agent is never a host binary. A node with no image runs on the host: a
//! deprecated fallback that journals `coder.host_spawn.deprecated`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx, ObsEvent};
use pii_graph::node::{GraphNode, NodeFactory};
use pii_observe::{Event, Level, Observer, names};
use sha2::{Digest, Sha256};

use super::agent_worker::{CriterionRun, Verdict, WorkerSpec, decide, plan};
use super::coder_container::{self, CONTAINER_WORKDIR, ContainerRun, PODMAN_BIN};
use super::coder_skills::{self, Drift, NoSkills, Rendered, SkillRefusal, SkillSource};
use super::criteria::{self, Table};

/// Where the agent binaries live in production. A coder node may exec `pi` and `claude` from here, and nothing
/// else new.
pub const AGENT_BIN_DIR: &str = "/root/.local/bin";

/// The agent program paths under `bin_dir`: the only programs a `coder` node may exec (spec 207).
#[must_use]
pub fn agent_binaries(bin_dir: &Path) -> Vec<String> {
    ["pi", "claude"]
        .iter()
        .map(|agent| bin_dir.join(agent).display().to_string())
        .collect()
}

/// The programs a run's scope admits (spec 207, 209, 215, 223): `/bin/sh`, the agent binaries, podman and the
/// criteria table's programs.
///
/// Dispatch hands each of them to a `coder` node only, and a node names only the one it execs.
#[must_use]
pub fn run_programs(bin_dir: &Path, podman: &Path, criteria: &Table) -> Vec<String> {
    let mut argv0 = vec![String::from("/bin/sh")];
    argv0.extend(agent_binaries(bin_dir));
    argv0.push(podman.display().to_string());
    argv0.extend(criteria::programs(criteria));
    argv0
}

/// How long one criterion command may run before it is killed.
const CRITERION_TIMEOUT: Duration = Duration::from_mins(10);

/// How often a running agent is checked for cancellation and the run's deadline.
const POLL: Duration = Duration::from_millis(50);

/// How long the image probe may run before the node refuses (spec 209).
const PROBE_TIMEOUT: Duration = Duration::from_mins(1);

/// Factory for `coder` nodes. The agent bin directory, the podman binary and the environment snapshot are fixed
/// at construction: production uses [`CoderFactory::new`]; tests inject their own.
pub struct CoderFactory {
    bin_dir: PathBuf,
    env: BTreeMap<String, String>,
    podman: PathBuf,
    criteria: Table,
    log: Arc<dyn Observer>,
    skills: Arc<dyn SkillSource>,
    runs_dir: Option<PathBuf>,
}

impl CoderFactory {
    /// The production factory: agents under [`AGENT_BIN_DIR`], podman at [`PODMAN_BIN`], names copied from this
    /// process's environment.
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
        Self {
            bin_dir,
            env,
            podman: PathBuf::from(PODMAN_BIN),
            criteria: criteria::production(),
            log,
            skills: Arc::new(NoSkills),
            runs_dir: None,
        }
    }

    /// Points container runs at `podman`; production keeps [`PODMAN_BIN`].
    #[must_use]
    pub fn with_podman(mut self, podman: PathBuf) -> Self {
        self.podman = podman;
        self
    }

    /// Replaces the criteria table; production keeps [`criteria::production`] (spec 215).
    #[must_use]
    pub fn with_criteria(mut self, criteria: Table) -> Self {
        self.criteria = criteria;
        self
    }

    /// Replaces the skill source. The daemon's registry passes the store (`StoreSkills`); the default is [`NoSkills`].
    #[must_use]
    pub fn with_skills(mut self, skills: Arc<dyn SkillSource>) -> Self {
        self.skills = skills;
        self
    }

    /// The directory a run's pinned skills are written under, as `<dir>/<run_id>/skills` (spec 225 R3).
    #[must_use]
    pub fn with_runs_dir(mut self, dir: PathBuf) -> Self {
        self.runs_dir = Some(dir);
        self
    }
}

/// A built `coder` node. Its config is read at run time, so a bad config is a refusal, not a build error.
struct CoderNode {
    bin_dir: PathBuf,
    env: BTreeMap<String, String>,
    podman: PathBuf,
    criteria: Table,
    log: Arc<dyn Observer>,
    skills: Arc<dyn SkillSource>,
    runs_dir: Option<PathBuf>,
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

/// The verdict of a node with no criteria of its own (spec 208): a clean exit hands on to a downstream `judge`,
/// and a non-zero exit goes to `err`, so the caller records it rather than the run failing outright.
fn deferred_verdict(code: i32, pid: u32, agent: &str, model: &str) -> Outcome {
    if code == 0 {
        Outcome::Next {
            port: PortName::from("ok"),
            value: serde_json::json!({ "pid": pid, "rc": code, "criteria": [], "deferred": true }),
        }
    } else {
        refused_port(agent, model, &format!("worker exited {code}"))
    }
}

/// The `err` port a refusal routes to: the scheduler never retries a refusal (spec 207 R6).
fn refused_port(agent: &str, model: &str, reason: &str) -> Outcome {
    Outcome::Next {
        port: PortName::from("err"),
        value: serde_json::json!({ "agent": agent, "model": model, "reason": reason }),
    }
}

/// What a launch came to: the agent's exit code, pid, duration and stop reason, or the reason the node refused to
/// launch.
enum Launch {
    Exited {
        code: i32,
        pid: u32,
        dur_ms: i64,
        stop: &'static str,
    },
    Refused(String),
}

/// A refusal `plan` already logged to the contract log: journalled for the run, then routed to `err` (spec 217 R5).
fn refuse_planned(ctx: &Ctx, agent: &str, model: &str, reason: &str) -> Outcome {
    journal(
        ctx,
        &Event::new("coder", Level::Warn, names::WORKER_START_REFUSED)
            .field("agent", agent.to_owned())
            .field("model", model.to_owned())
            .field("reason", reason.to_owned()),
    );
    refused_port(agent, model, reason)
}

/// Reports an agent-call event to the run's context, where the engine records it in the journal (spec 217).
fn journal(ctx: &Ctx, event: &Event) {
    ctx.observer().event(&ObsEvent::NodeObserved {
        kind: event.kind.to_owned(),
        fields: serde_json::to_value(&event.fields).unwrap_or_default(),
    });
}

/// The stop reason of a finished child (spec 217 R4). A signal death is `128 + signal`, so a code above 128 is
/// `signalled`; any other code is the agent ending on its own.
fn stop_reason(code: i32) -> &'static str {
    if code > 128 { "signalled" } else { "end_turn" }
}

/// Milliseconds since `started`, saturating: a duration never wraps negative.
fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// The sha256 of an argv as lowercase hex (spec 217 R3): identifies a call without journalling its arguments.
fn argv_digest(argv: &[String]) -> String {
    let digest = Sha256::digest(argv.join("\0").as_bytes());
    format!("{digest:x}")
}

/// The pinned skills' names, comma-joined (spec 225 R7).
fn skill_names(skills: &[Rendered]) -> String {
    skills
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The pinned skills' sha256s, comma-joined, in the order of [`skill_names`] (spec 225 R7).
fn skill_hashes(skills: &[Rendered]) -> String {
    skills
        .iter()
        .map(|s| s.sha256.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// The session an agent was launched with: the value after `--session`, empty when the argv names none.
fn session_of(argv: &[String]) -> String {
    argv.iter()
        .position(|a| a == "--session")
        .and_then(|i| argv.get(i + 1))
        .cloned()
        .unwrap_or_default()
}

impl CoderNode {
    /// Reports an agent-call event to the contract log, and to the run's context so the engine journals it (spec 217).
    fn observe(&self, ctx: &Ctx, event: &Event) {
        self.log.event(event);
        journal(ctx, event);
    }

    /// Journals a refusal the node decided itself, then routes it to `err`.
    fn refuse(&self, ctx: &Ctx, agent: &str, model: &str, reason: &str) -> Outcome {
        self.refuse_with(ctx, agent, model, reason, None)
    }

    /// Journals a refusal with the image digest slot it had (spec 223 R3), then routes it to `err`.
    fn refuse_with(
        &self,
        ctx: &Ctx,
        agent: &str,
        model: &str,
        reason: &str,
        image_digest: Option<&str>,
    ) -> Outcome {
        let mut event = Event::new("coder", Level::Warn, names::WORKER_START_REFUSED)
            .field("agent", agent.to_owned())
            .field("model", model.to_owned())
            .field("reason", reason.to_owned());
        if let Some(digest) = image_digest {
            event = event.field("image_digest", digest.to_owned());
        }
        self.observe(ctx, &event);
        refused_port(agent, model, reason)
    }

    /// Journals a criterion the table does not name (spec 215), then fails the node with it (spec 224). Nothing was
    /// spawned.
    fn refuse_criterion(&self, criterion: &str, agent: &str, model: &str) -> Outcome {
        self.log.event(
            &Event::new("coder", Level::Warn, names::CRITERIA_UNKNOWN)
                .field("criterion", criterion.to_owned())
                .field("agent", agent.to_owned())
                .field("model", model.to_owned()),
        );
        Outcome::Fail {
            error: format!("criteria.unknown: '{criterion}' is not in the criteria table"),
        }
    }

    /// Each name in the node's `criteria` with its fixed argv; the first name the table lacks is the error.
    fn resolve_criteria(
        &self,
        cfg: &serde_json::Value,
    ) -> Result<Vec<(String, Vec<String>)>, String> {
        texts(cfg, "criteria")
            .into_iter()
            .map(|name| match self.criteria.get(&name) {
                Some(argv) => Ok((name.clone(), argv.clone())),
                None => Err(name),
            })
            .collect()
    }

    /// Journals a refused image (spec 209). The image is never built or pulled here.
    fn refuse_image(&self, image: &str, reason: &str) -> Launch {
        self.log.event(
            &Event::new("coder", Level::Warn, names::CODER_IMAGE_MISSING)
                .field("image", image.to_owned())
                .field("reason", reason.to_owned()),
        );
        Launch::Refused(reason.to_owned())
    }

    /// The daemon's values for exactly the names in `names`; a name the daemon lacks is simply absent.
    fn pass_env(&self, names: &[String]) -> BTreeMap<String, String> {
        names
            .iter()
            .filter_map(|n| self.env.get(n).map(|v| (n.clone(), v.clone())))
            .collect()
    }

    /// Runs the agent inside `image` (spec 209). The image must already exist and carry a digest, which is probed
    /// and journalled; the env file is removed once the run has exited, on every path.
    fn launch_in_container(
        &self,
        ctx: &Ctx,
        image: &str,
        spec: &WorkerSpec,
        env: BTreeMap<String, String>,
        argv: &[String],
        skills: &[Rendered],
    ) -> Result<Launch, PiiError> {
        if coder_container::forbidden_mount(&spec.workdir) {
            return Ok(Launch::Refused(format!(
                "workspace mount refused: {}",
                spec.workdir
            )));
        }
        let podman = self.podman.display().to_string();
        let workdir = PathBuf::from(&spec.workdir);
        let probe = [
            podman.clone(),
            "image".to_owned(),
            "inspect".to_owned(),
            "--format".to_owned(),
            "{{.Digest}}".to_owned(),
            image.to_owned(),
        ];
        let digest = match ctx.proc()?.run(&probe, &workdir, PROBE_TIMEOUT) {
            Ok(out) if out.exit_code == 0 => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            Ok(_) => return Ok(self.refuse_image(image, "image not present; build it outside pii")),
            Err(e) => return Ok(Launch::Refused(format!("{e}"))),
        };
        if digest.is_empty() {
            return Ok(self.refuse_image(image, "image has no digest; pin it by digest"));
        }
        let skills_dir = match self.materialise(ctx, skills) {
            Ok(dir) => dir.map(|p| p.display().to_string()),
            Err(reason) => return Ok(Launch::Refused(reason)),
        };

        let env_file = match self.env_file_path() {
            Ok(path) => path,
            Err(reason) => return Ok(Launch::Refused(reason)),
        };
        let pairs: Vec<(String, String)> = env.into_iter().collect();
        if let Err(e) = coder_container::write_env_file(&env_file, &pairs) {
            return Ok(Launch::Refused(format!("env file: {e}")));
        }
        let run_id = ctx.trace().run.as_str().to_owned();
        let network = self.network_grant();
        let env_file_str = env_file.display().to_string();
        let run = ContainerRun {
            podman: &podman,
            image,
            workdir: &spec.workdir,
            env_file: &env_file_str,
            network: &network,
            run_id: &run_id,
            skills_dir: skills_dir.as_deref(),
        };
        let run_argv = coder_container::run_argv(&run, argv);
        // podman's own environment: the daemon's PATH and HOME, never a secret.
        let podman_env: Vec<(String, String)> = ["PATH", "HOME"]
            .iter()
            .filter_map(|k| self.env.get(*k).map(|v| ((*k).to_owned(), v.clone())))
            .collect();
        let result = self.spawn_and_wait(
            ctx,
            spec,
            &run_argv,
            &podman_env,
            Some((image, digest.as_str())),
            skills,
        );
        let _ = std::fs::remove_file(&env_file);
        result
    }

    /// Journals `skill.drift` and refuses the node before anything spawns (spec 225 R4).
    fn refuse_drift(&self, ctx: &Ctx, agent: &str, model: &str, drift: &Drift) -> Outcome {
        self.observe(
            ctx,
            &Event::new("coder", Level::Warn, names::SKILL_DRIFT)
                .field("name", drift.name.clone())
                .field("pinned", drift.pinned.clone())
                .field("found", drift.found.clone()),
        );
        let reason = format!(
            "skill.drift: {} pinned {} found {}",
            drift.name, drift.pinned, drift.found
        );
        self.refuse(ctx, agent, model, &reason)
    }

    /// Writes the run's skills under `<runs_dir>/<run_id>/skills` (spec 225 R3). `None` when the run pins none.
    fn materialise(&self, ctx: &Ctx, skills: &[Rendered]) -> Result<Option<PathBuf>, String> {
        if skills.is_empty() {
            return Ok(None);
        }
        let Some(runs) = &self.runs_dir else {
            return Err(String::from("skills: no runs directory is configured"));
        };
        let run_dir = runs.join(ctx.trace().run.as_str());
        coder_skills::materialise(&run_dir, skills)
            .map(Some)
            .map_err(|e| format!("skills: {e}"))
    }

    /// A fresh env-file path under the runs dir, unique within this process (spec 227 R6). The file is created with
    /// `create_new`, so an existing path is an error, never overwritten. No runs dir is a refusal: the secrets have
    /// nowhere private to go.
    fn env_file_path(&self) -> Result<PathBuf, String> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let Some(runs) = &self.runs_dir else {
            return Err(String::from("env file: no runs directory is configured"));
        };
        std::fs::create_dir_all(runs).map_err(|e| format!("env file: {e}"))?;
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        Ok(runs.join(format!("pii-coder-{}-{seq}.env", std::process::id())))
    }

    /// The network a container run gets: `none`, unless the graph declares a grant (spec 209).
    fn network_grant(&self) -> String {
        text(&self.cfg, "network")
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "none".to_owned())
    }

    /// Runs the agent on the host: the deprecated fallback for a node with no `container_image` (spec 207).
    fn launch_on_host(
        &self,
        ctx: &Ctx,
        argv: &[String],
        spec: &WorkerSpec,
        env: BTreeMap<String, String>,
    ) -> Result<Launch, PiiError> {
        let Some((program, rest)) = argv.split_first() else {
            return Ok(Launch::Refused("empty agent argv".to_owned()));
        };
        let mut host_argv = vec![self.bin_dir.join(program).display().to_string()];
        host_argv.extend(rest.iter().cloned());
        let env_pairs: Vec<(String, String)> = env.into_iter().collect();
        self.spawn_and_wait(ctx, spec, &host_argv, &env_pairs, None, &[])
    }

    /// Journals a finished call (spec 217 R4). `runs` are the criteria that ran; none for a deferred verdict.
    fn emit_exited(
        &self,
        ctx: &Ctx,
        pid: u32,
        rc: i32,
        dur_ms: i64,
        stop: &str,
        runs: &[CriterionRun],
    ) {
        let failed = runs.iter().find(|r| r.rc != 0);
        let passed = runs.iter().filter(|r| r.rc == 0).count();
        self.observe(
            ctx,
            &Event::new("coder", Level::Info, names::CODER_EXITED)
                .field("pid", i64::from(pid))
                .field("rc", i64::from(rc))
                .field("dur_ms", dur_ms)
                .field("stop_reason", stop.to_owned())
                .field(
                    "criteria_run",
                    i64::try_from(runs.len()).unwrap_or(i64::MAX),
                )
                .field("criteria_passed", i64::try_from(passed).unwrap_or(i64::MAX))
                .field(
                    "criteria_failed",
                    i64::try_from(runs.len() - passed).unwrap_or(i64::MAX),
                )
                .field(
                    "first_failure",
                    failed.map(|r| r.command.clone()).unwrap_or_default(),
                ),
        );
    }

    /// Spawns `argv` with exactly `env_pairs` in the spec's workspace, journals `coder.spawned` and
    /// `coder.env.names`, and waits for it (spec 217). Its output is drained. A cancelled or timed-out run kills
    /// the child and journals it as `killed`. A container run passes the image and digest it used (spec 209).
    fn spawn_and_wait(
        &self,
        ctx: &Ctx,
        spec: &WorkerSpec,
        argv: &[String],
        env_pairs: &[(String, String)],
        container: Option<(&str, &str)>,
        skills: &[Rendered],
    ) -> Result<Launch, PiiError> {
        let started = Instant::now();
        let workdir = Path::new(&spec.workdir);
        let mut child = ctx.proc()?.spawn_child_exact(argv, workdir, env_pairs)?;
        let pid = child.pid();
        let (image, image_digest) = container.unwrap_or_default();
        self.observe(
            ctx,
            &Event::new("coder", Level::Info, names::CODER_SPAWNED)
                .field("agent", spec.agent_name().to_owned())
                .field("model", spec.model.clone())
                .field("image", image.to_owned())
                .field("image_digest", image_digest.to_owned())
                .field("argv0", argv.first().cloned().unwrap_or_default())
                .field("argv_len", i64::try_from(argv.len()).unwrap_or(i64::MAX))
                .field("argv_digest", argv_digest(argv))
                .field("pid", i64::from(pid))
                .field("session", session_of(argv))
                .field("skill", skill_names(skills))
                .field("sha256", skill_hashes(skills)),
        );
        let env_names: Vec<&str> = env_pairs.iter().map(|(k, _)| k.as_str()).collect();
        self.log.event(
            &Event::new("coder", Level::Info, names::CODER_ENV_NAMES)
                .field("names", env_names.join(",")),
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
                self.emit_exited(ctx, pid, -1, elapsed_ms(started), "killed", &[]);
                return Err(e);
            }
        };
        Ok(Launch::Exited {
            code,
            pid,
            dur_ms: elapsed_ms(started),
            stop: stop_reason(code),
        })
    }
}

/// The node's config, with `spec` taken from the item `select` claimed when the config does not name one (spec 208):
/// the queue graph is static, so the item it dispatches is only known at run time.
fn with_selected_spec(cfg: &serde_json::Value, input: &NodeInput) -> serde_json::Value {
    let mut cfg = cfg.clone();
    let selected = input
        .nodes
        .get("select")
        .and_then(|v| v.get("item"))
        .and_then(|i| i.get("spec"))
        .cloned();
    if let (Some(map), Some(spec)) = (cfg.as_object_mut(), selected) {
        map.entry("spec").or_insert(spec);
    }
    cfg
}

impl GraphNode for CoderNode {
    #[allow(clippy::too_many_lines)] // reason: the node's refusals, spawn and criteria decide in one linear sequence
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let cfg = with_selected_spec(&self.cfg, input);
        let cfg = &cfg;
        let agent = text(cfg, "agent").unwrap_or_default();
        let model = text(cfg, "model").unwrap_or_default();
        let (spec, brief) = match (WorkerSpec::from_cfg(cfg), text(cfg, "brief")) {
            (Ok(spec), Some(brief)) if !spec.spec.is_empty() => (spec, brief),
            (Err(e), _) => return Ok(self.refuse(ctx, &agent, &model, &format!("{e}"))),
            _ => return Ok(self.refuse(ctx, &agent, &model, "no spec or no brief")),
        };
        // Spec 215: every criterion resolves to its fixed argv before anything spawns; an unknown name refuses.
        let checks = match self.resolve_criteria(cfg) {
            Ok(checks) => checks,
            Err(name) => return Ok(self.refuse_criterion(&name, &agent, &model)),
        };
        let env = self.pass_env(&texts(cfg, "env_pass"));
        let image = text(cfg, "container_image").filter(|i| !i.is_empty());
        // Spec 225 R4: every pinned skill is checked against the store before anything spawns.
        let skills = match coder_skills::check(self.skills.as_ref(), cfg) {
            Ok(skills) => skills,
            Err(SkillRefusal::Drift(drift)) => {
                return Ok(self.refuse_drift(ctx, &agent, &model, &drift));
            }
            Err(SkillRefusal::Invalid(reason)) => {
                return Ok(self.refuse(ctx, &agent, &model, &reason));
            }
        };
        if !skills.is_empty() && image.is_none() {
            let reason =
                "skills need a container_image: the host fallback cannot bind them read-only";
            return Ok(self.refuse(ctx, &agent, &model, reason));
        }
        // Inside a container the workspace is at `/work`, so the planned session path must be too.
        let planned = match &image {
            Some(_) => WorkerSpec {
                workdir: CONTAINER_WORKDIR.to_owned(),
                ..spec.clone()
            },
            None => spec.clone(),
        };
        let dispatch = match plan(&planned, &brief, &env, "direct", self.log.as_ref()) {
            Ok(dispatch) => dispatch,
            Err(e) => return Ok(refuse_planned(ctx, &agent, &model, &format!("{e}"))),
        };
        let launch = if let Some(image) = &image {
            self.launch_in_container(ctx, image, &spec, env, &dispatch.argv, &skills)?
        } else {
            self.log.event(
                &Event::new("coder", Level::Warn, names::CODER_HOST_SPAWN_DEPRECATED)
                    .field("agent", agent.clone()),
            );
            self.launch_on_host(ctx, &dispatch.argv, &spec, env)?
        };
        let (code, pid, dur_ms, stop) = match launch {
            Launch::Exited {
                code,
                pid,
                dur_ms,
                stop,
            } => (code, pid, dur_ms, stop),
            // A refusal is journalled before it routes (spec 217 R5): no pid, and no `coder.exited`. No spawn ran, so an
            // image node's digest slot reads `missing` (spec 223 R3).
            Launch::Refused(reason) => {
                let digest = image.as_ref().map(|_| "missing");
                return Ok(self.refuse_with(ctx, &agent, &model, &reason, digest));
            }
        };
        let workdir = PathBuf::from(&spec.workdir);

        // The criteria decide. The first failing criterion stops the run and names itself in the verdict.
        let mut runs = Vec::new();
        for (command, argv) in checks {
            let rc = ctx
                .proc()?
                .run(&argv, &workdir, CRITERION_TIMEOUT)?
                .exit_code;
            runs.push(CriterionRun { command, rc });
            if rc != 0 {
                break;
            }
        }
        self.emit_exited(ctx, pid, code, dur_ms, stop, &runs);

        // No criteria configured: the verdict is deferred to a downstream `judge` node (spec 208). A clean exit
        // hands on; a non-zero exit goes to `err`, so the caller records it rather than the run failing outright.
        if texts(cfg, "criteria").is_empty() {
            return Ok(deferred_verdict(code, pid, &agent, &model));
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

    /// The host agents (the deprecated fallback), podman and the criteria table's programs: nothing else new.
    fn proc_argv0_grant(&self) -> Vec<String> {
        let mut grant = agent_binaries(&self.bin_dir);
        grant.push(self.podman.display().to_string());
        grant.extend(criteria::programs(&self.criteria));
        grant
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(CoderNode {
            bin_dir: self.bin_dir.clone(),
            env: self.env.clone(),
            podman: self.podman.clone(),
            criteria: self.criteria.clone(),
            log: Arc::clone(&self.log),
            skills: Arc::clone(&self.skills),
            runs_dir: self.runs_dir.clone(),
            cfg: cfg.clone(),
        }))
    }
}
