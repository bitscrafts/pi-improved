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
use std::time::Duration;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::node::{GraphNode, NodeFactory};
use pii_observe::{Event, Level, Observer, names};

use super::agent_worker::{CriterionRun, Verdict, WorkerSpec, decide, plan};
use super::coder_container::{self, CONTAINER_WORKDIR, ContainerRun, PODMAN_BIN};

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
    log: Arc<dyn Observer>,
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
            log,
        }
    }

    /// Points container runs at `podman`; production keeps [`PODMAN_BIN`].
    #[must_use]
    pub fn with_podman(mut self, podman: PathBuf) -> Self {
        self.podman = podman;
        self
    }
}

/// A built `coder` node. Its config is read at run time, so a bad config is a refusal, not a build error.
struct CoderNode {
    bin_dir: PathBuf,
    env: BTreeMap<String, String>,
    podman: PathBuf,
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

/// What a launch came to: the agent's exit code and pid, or the reason the node refused to launch.
enum Launch {
    Exited { code: i32, pid: u32 },
    Refused(String),
}

/// A fresh env-file path under the temp dir, unique within this process. The file is created with `create_new`,
/// so an existing path is an error, never overwritten.
fn env_file_path() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pii-coder-{}-{seq}.env", std::process::id()))
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

        let env_file = env_file_path();
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
        };
        let run_argv = coder_container::run_argv(&run, argv);
        // podman's own environment: the daemon's PATH and HOME, never a secret.
        let podman_env: Vec<(String, String)> = ["PATH", "HOME"]
            .iter()
            .filter_map(|k| self.env.get(*k).map(|v| ((*k).to_owned(), v.clone())))
            .collect();
        let result = self.spawn_and_wait(
            ctx,
            &run_argv,
            &workdir,
            &podman_env,
            Some((image, digest.as_str())),
        );
        let _ = std::fs::remove_file(&env_file);
        result
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
        self.spawn_and_wait(ctx, &host_argv, Path::new(&spec.workdir), &env_pairs, None)
    }

    /// Spawns `argv` with exactly `env_pairs`, journals `coder.spawned`, and waits for it. Its output is drained,
    /// and a cancelled or timed-out run kills the child. `container` names the image and digest a container run
    /// used, so the journal records which image produced the run (spec 209).
    fn spawn_and_wait(
        &self,
        ctx: &Ctx,
        argv: &[String],
        workdir: &Path,
        env_pairs: &[(String, String)],
        container: Option<(&str, &str)>,
    ) -> Result<Launch, PiiError> {
        let mut child = ctx.proc()?.spawn_child_exact(argv, workdir, env_pairs)?;
        let pid = child.pid();
        let mut spawned = Event::new("coder", Level::Info, names::CODER_SPAWNED)
            .field("pid", i64::from(pid))
            .field("argv", argv.join(" "));
        if let Some((image, digest)) = container {
            spawned = spawned
                .field("image", image.to_owned())
                .field("image_digest", digest.to_owned());
        }
        self.log.event(&spawned);
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
        Ok(Launch::Exited { code, pid })
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
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let cfg = with_selected_spec(&self.cfg, input);
        let cfg = &cfg;
        let agent = text(cfg, "agent").unwrap_or_default();
        let model = text(cfg, "model").unwrap_or_default();
        let (spec, brief) = match (WorkerSpec::from_cfg(cfg), text(cfg, "brief")) {
            (Ok(spec), Some(brief)) if !spec.spec.is_empty() => (spec, brief),
            (Err(e), _) => return Ok(self.refuse(&agent, &model, &format!("{e}"))),
            _ => return Ok(self.refuse(&agent, &model, "no spec or no brief")),
        };
        let env = self.pass_env(&texts(cfg, "env_pass"));
        let image = text(cfg, "container_image").filter(|i| !i.is_empty());
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
            Err(e) => return Ok(refused_port(&agent, &model, &format!("{e}"))),
        };
        let launch = if let Some(image) = &image {
            self.launch_in_container(ctx, image, &spec, env, &dispatch.argv)?
        } else {
            self.log.event(
                &Event::new("coder", Level::Warn, names::CODER_HOST_SPAWN_DEPRECATED)
                    .field("agent", agent.clone()),
            );
            self.launch_on_host(ctx, &dispatch.argv, &spec, env)?
        };
        let (code, pid) = match launch {
            Launch::Exited { code, pid } => (code, pid),
            Launch::Refused(reason) => return Ok(refused_port(&agent, &model, &reason)),
        };
        let workdir = PathBuf::from(&spec.workdir);

        // No criteria configured: the verdict is deferred to a downstream `judge` node (spec 208). A clean exit
        // hands on; a non-zero exit goes to `err`, so the caller records it rather than the run failing outright.
        if texts(cfg, "criteria").is_empty() {
            return Ok(deferred_verdict(code, pid, &agent, &model));
        }

        // The criteria decide. The first failing criterion stops the run and names itself in the verdict.
        let mut runs = Vec::new();
        for command in texts(cfg, "criteria") {
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

    /// The host agents (the deprecated fallback) and podman: a coder node may exec those and nothing else new.
    fn proc_argv0_grant(&self) -> Vec<String> {
        let mut grant = agent_binaries(&self.bin_dir);
        grant.push(self.podman.display().to_string());
        grant
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(CoderNode {
            bin_dir: self.bin_dir.clone(),
            env: self.env.clone(),
            podman: self.podman.clone(),
            log: Arc::clone(&self.log),
            cfg: cfg.clone(),
        }))
    }
}
