//! Starting the agent: in a container (spec 209/230) or on the host (spec 207), plus the skill materialisation and
//! network grant a container run needs. Split from `coder.rs` for the 450-line gate; the methods are the same
//! `impl CoderNode`, so every caller is unchanged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pii_abi::{Outcome, PiiError};
use pii_core::Ctx;
use pii_observe::{Event, Level, names};

use super::super::agent_worker::WorkerSpec;
use super::super::coder_container;
use super::super::coder_runtime::{
    ContainerRuntime, Launch, PodmanApi, PodmanCli, Refusal, Request, Spawn, wait_child,
};
use super::super::coder_skills::{self, Drift, Rendered};
use super::{CoderNode, prepare_pi_session, text};

impl CoderNode {
    /// Runs the agent inside `image` through `runtime` (spec 209, 230). The workspace mount is checked first, then the
    /// skills are materialised; the env file is the CLI runtime's alone.
    #[allow(clippy::too_many_arguments)] // reason: the node's launch inputs, each taken from the graph or the run
    pub(super) fn launch_in_container(
        &self,
        ctx: &Ctx,
        timeout: Option<Duration>,
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
            return self.journalled_run(ctx, spec, skills, |spawned| {
                api.run(ctx, &request, timeout, spawned)
            });
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
        self.journalled_run(ctx, spec, skills, |spawned| {
            cli.run(ctx, &request, timeout, spawned)
        })
    }

    /// Journals `skill.drift` and refuses the node before anything spawns (spec 225 R4).
    pub(super) fn refuse_drift(
        &self,
        ctx: &Ctx,
        agent: &str,
        model: &str,
        drift: &Drift,
    ) -> Outcome {
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
    pub(super) fn materialise(
        &self,
        ctx: &Ctx,
        skills: &[Rendered],
    ) -> Result<Option<PathBuf>, String> {
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
    pub(super) fn network_grant(&self) -> String {
        text(&self.cfg, "network")
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "none".to_owned())
    }

    /// Runs the agent on the host: the deprecated fallback for a node with no `container_image` (spec 207).
    pub(super) fn launch_on_host(
        &self,
        ctx: &Ctx,
        argv: &[String],
        spec: &WorkerSpec,
        env: BTreeMap<String, String>,
        timeout: Option<Duration>,
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
                timeout,
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
}
