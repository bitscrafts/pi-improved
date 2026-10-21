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
//!
//! Spec 230: `container_runtime` names how the container is started: `cli` (the default, `podman run`) or `socket`
//! (the podman REST API over the user's runtime socket). The runtime itself is [`super::coder_runtime`].

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pii_abi::{Effects, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::node::{GraphNode, NodeFactory};
use pii_observe::{Event, Level, Observer, names};

use super::agent_worker::WorkerSpec;
use super::coder_container::{self, PODMAN_BIN};
use super::coder_runtime::{
    ContainerRuntime, Launch, PodmanApi, PodmanCli, Refusal, Request, Spawn, elapsed_ms, wait_child,
};
use super::coder_skills::{self, Drift, NoSkills, Rendered, SkillSource};
use super::criteria::{self, Table};

mod graph_node;
mod helpers;
mod paths;
mod report;

use self::helpers::{
    argv_digest, journal, refused_port, session_of, skill_hashes, skill_names, text, texts,
};
pub use self::paths::{agent_bin_dir, agent_binaries, prepare_pi_session, run_programs};
use self::report::RunReport;

/// How long one criterion command may run before it is killed.
pub(super) const CRITERION_TIMEOUT: Duration = Duration::from_mins(10);

/// The one scope-checked criterion name (spec 232 R1): its argv runs in the node's workdir, so the
/// node's workdir must be the run's scope root for the name to resolve.
const WORKSPACE_CRITERION: &str = "cargo:test:workspace/all";

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
    runtime_socket: Option<PathBuf>,
}

impl CoderFactory {
    /// The production factory: agents under [`agent_bin_dir`], podman at [`PODMAN_BIN`], names copied from this
    /// process's environment. With no agent directory on `PATH` the factory has none, so a `coder` refuses.
    #[must_use]
    pub fn new(log: Arc<dyn Observer>) -> Self {
        Self::with_env(
            agent_bin_dir().unwrap_or_default(),
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
            runtime_socket: None,
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

    /// Points the `socket` runtime at `socket`; production resolves `XDG_RUNTIME_DIR/podman/podman.sock` (spec 230).
    #[must_use]
    pub fn with_runtime_socket(mut self, socket: PathBuf) -> Self {
        self.runtime_socket = Some(socket);
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
    runtime_socket: Option<PathBuf>,
    cfg: serde_json::Value,
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
    /// The table is fixed, except one name the RESOLVE checks: `cargo:test:workspace/all` (spec 232 R1)
    /// runs `cargo test` in whatever workdir the node names, so the node's `workdir` must be the run's
    /// scope root (the run input's `scope_root`, absent = the daemon workspace the run already confined).
    /// The fixed table cannot be extended by graph data; the checked name keeps the check honest.
    fn resolve_criteria(
        &self,
        cfg: &serde_json::Value,
        scope_root: Option<&str>,
    ) -> Result<Vec<(String, Vec<String>)>, String> {
        let workdir = text(cfg, "workdir").unwrap_or_default();
        texts(cfg, "criteria")
            .into_iter()
            .map(|name| match self.criteria.get(&name) {
                Some(argv) => {
                    if name == WORKSPACE_CRITERION
                        && scope_root.is_some_and(|root| {
                            !Path::new(root).starts_with(&workdir)
                                && !Path::new(&workdir).starts_with(root)
                        })
                    {
                        return Err(name);
                    }
                    Ok((name.clone(), argv.clone()))
                }
                None => Err(name),
            })
            .collect()
    }

    /// Journals the event a refusal names, before the refusal routes (spec 209, 230 R3). A refusal with no event of
    /// its own is journalled by `worker.start.refused` alone.
    fn journal_refusal(&self, refusal: &Refusal) {
        match refusal {
            Refusal::Image { image, reason } => self.log.event(
                &Event::new("coder", Level::Warn, names::CODER_IMAGE_MISSING)
                    .field("image", image.clone())
                    .field("reason", reason.clone())
                    // Spec 228 R2: podman's own words, trimmed to 240 chars, so the journal can
                    // never replace the runtime's failure with an invented cause.
                    .field("probe_stderr", reason.clone()),
            ),
            Refusal::Runtime {
                socket,
                status,
                reason,
            } => {
                let mut event = Event::new("coder", Level::Warn, names::CODER_RUNTIME_UNAVAILABLE)
                    .field("socket", socket.clone())
                    .field("reason", reason.clone());
                if let Some(status) = status {
                    event = event.field("status", i64::from(*status));
                }
                self.log.event(&event);
            }
            Refusal::Other(_) => {}
        }
    }

    /// The daemon's values for exactly the names in `names`; a name the daemon lacks is simply absent.
    fn pass_env(&self, names: &[String]) -> BTreeMap<String, String> {
        names
            .iter()
            .filter_map(|n| self.env.get(n).map(|v| (n.clone(), v.clone())))
            .collect()
    }

    /// Runs the agent inside `image` through `runtime` (spec 209, 230). The workspace mount is checked first, then the
    /// skills are materialised; the env file is the CLI runtime's alone.
    #[allow(clippy::too_many_arguments)] // reason: the node's launch inputs, each taken from the graph or the run
    fn launch_in_container(
        &self,
        ctx: &Ctx,
        image: &str,
        spec: &WorkerSpec,
        env: BTreeMap<String, String>,
        argv: &[String],
        skills: &[Rendered],
        runtime: &str,
    ) -> Result<Launch, PiiError> {
        if coder_container::forbidden_mount(&spec.workdir) {
            return Ok(Launch::Refused(Refusal::Other(format!(
                "workspace mount refused: {}",
                spec.workdir
            ))));
        }
        let skills_dir = match self.materialise(ctx, skills) {
            Ok(dir) => dir.map(|p| p.display().to_string()),
            Err(reason) => return Ok(Launch::Refused(Refusal::Other(reason))),
        };
        let pairs: Vec<(String, String)> = env.into_iter().collect();
        let run_id = ctx.trace().run.as_str().to_owned();
        let network = self.network_grant();
        let request = Request {
            image,
            workdir: &spec.workdir,
            network: &network,
            run_id: &run_id,
            skills_dir: skills_dir.as_deref(),
            agent_argv: argv,
            env: &pairs,
        };
        if runtime == "socket" {
            let socket = match &self.runtime_socket {
                Some(socket) => socket.clone(),
                None => match pii_tools::sandbox::runtime_socket() {
                    Ok(socket) => socket,
                    Err(e) => {
                        return Ok(Launch::Refused(Refusal::Runtime {
                            socket: String::new(),
                            status: None,
                            reason: format!("{e}"),
                        }));
                    }
                },
            };
            let api = PodmanApi { socket };
            return self
                .journalled_run(ctx, spec, skills, |spawned| api.run(ctx, &request, spawned));
        }
        // podman's own environment: the daemon's PATH and HOME, never a secret.
        let podman_env: Vec<(String, String)> = ["PATH", "HOME"]
            .iter()
            .filter_map(|k| self.env.get(*k).map(|v| ((*k).to_owned(), v.clone())))
            .collect();
        let cli = PodmanCli {
            podman: self.podman.clone(),
            env: podman_env,
            runs_dir: self.runs_dir.clone(),
        };
        self.journalled_run(ctx, spec, skills, |spawned| cli.run(ctx, &request, spawned))
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
            return Ok(Launch::Refused(Refusal::Other(
                "empty agent argv".to_owned(),
            )));
        };
        // pi refuses a missing or empty session file, so the header is written before the spawn. A resumed
        // session is left as it is. A failure to write it is a refusal, never a spawn into a file pi rejects.
        if let Err(refusal) = prepare_pi_session(spec, ctx.now_ms()) {
            return Ok(Launch::Refused(refusal));
        }
        let mut host_argv = vec![self.bin_dir.join(program).display().to_string()];
        host_argv.extend(rest.iter().cloned());
        let env_pairs: Vec<(String, String)> = env.into_iter().collect();
        self.journalled_run(ctx, spec, &[], |spawned| {
            wait_child(
                ctx,
                &host_argv,
                Path::new(&spec.workdir),
                &env_pairs,
                &mut |pid| {
                    spawned(&Spawn {
                        runtime: "host",
                        image: String::new(),
                        image_digest: String::new(),
                        container: String::new(),
                        pid,
                        argv: host_argv.clone(),
                        env_names: env_pairs.iter().map(|(k, _)| k.clone()).collect(),
                    });
                },
            )
        })
    }

    /// Journals a finished call (spec 217 R4). `report.runs` are the criteria that ran; none for a deferred verdict.
    fn emit_exited(
        &self,
        ctx: &Ctx,
        pid: u32,
        rc: i32,
        dur_ms: i64,
        stop: &str,
        report: &RunReport<'_>,
    ) {
        let report_event = report.exited_event(pid, rc, dur_ms, stop);
        self.observe(ctx, &report_event);
    }

    /// Runs `run`, journalling `coder.spawned` (and `coder.env.names`) as the runtime reports the pid, before it waits
    /// (spec 217). A run cancelled or timed out after its spawn journals `coder.exited` as `killed`, with the pid it
    /// had.
    fn journalled_run(
        &self,
        ctx: &Ctx,
        spec: &WorkerSpec,
        skills: &[Rendered],
        run: impl FnOnce(&mut dyn FnMut(&Spawn)) -> Result<Launch, PiiError>,
    ) -> Result<Launch, PiiError> {
        let started = Instant::now();
        let spawned_pid = Cell::new(None);
        let result = run(&mut |s: &Spawn| {
            spawned_pid.set(Some(s.pid));
            self.journal_spawned(ctx, spec, skills, s);
        });
        if let (Err(PiiError::Cancelled | PiiError::Timeout { .. }), Some(pid)) =
            (&result, spawned_pid.get())
        {
            let empty = RunReport {
                runs: &[],
                failure_tail: "",
                routes: &[],
                agent_output: "",
                agent_stderr: "",
            };
            self.emit_exited(ctx, pid, -1, elapsed_ms(started), "killed", &empty);
        }
        result
    }

    /// Journals `coder.spawned` and `coder.env.names` for a run that has its pid (spec 217, 209, 230). Names only:
    /// no argv value and no env value.
    fn journal_spawned(&self, ctx: &Ctx, spec: &WorkerSpec, skills: &[Rendered], s: &Spawn) {
        self.observe(
            ctx,
            &Event::new("coder", Level::Info, names::CODER_SPAWNED)
                .field("agent", spec.agent_name().to_owned())
                .field("model", spec.model.clone())
                .field("runtime", s.runtime.to_owned())
                .field("image", s.image.clone())
                .field("image_digest", s.image_digest.clone())
                .field("container", s.container.clone())
                .field("argv0", s.argv.first().cloned().unwrap_or_default())
                .field("argv_len", i64::try_from(s.argv.len()).unwrap_or(i64::MAX))
                .field("argv_digest", argv_digest(&s.argv))
                .field("pid", i64::from(s.pid))
                .field("session", session_of(&s.argv))
                .field("skill", skill_names(skills))
                .field("sha256", skill_hashes(skills)),
        );
        self.log.event(
            &Event::new("coder", Level::Info, names::CODER_ENV_NAMES)
                .field("names", s.env_names.join(",")),
        );
    }
}

/// The node's config, with `spec` taken from the item `select` claimed when the config does not name one (spec 208):
/// the queue graph is static, so the item it dispatches is only known at run time.
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
            runtime_socket: self.runtime_socket.clone(),
            cfg: cfg.clone(),
        }))
    }
}
