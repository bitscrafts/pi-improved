//! The spec 209 container runtime: `podman run` as a child process of the daemon. Split out of
//! `coder_runtime.rs` by the 450-line gate; the socket runtime (spec 230) stays there.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pii_abi::PiiError;
use pii_core::Ctx;
use serde_json::Value;

use super::coder_container::{self, ContainerRun};
use super::coder_runtime::{
    ContainerRuntime, Launch, Listed, PROBE_TIMEOUT, Refusal, Request, Spawn, wait_child,
};
use super::coder_runtime_util::{env_file_path, env_names, probe_reason, probe_says_absent};

/// The spec 209 runtime: the image is probed, then `podman run` is spawned as a child process.
pub struct PodmanCli {
    /// The podman binary the run execs.
    pub podman: PathBuf,
    /// The daemon's PATH and HOME: the only environment podman itself is given.
    pub env: Vec<(String, String)>,
    /// The runs directory the env file is written under; `None` is a refusal once the image has been probed.
    pub runs_dir: Option<PathBuf>,
}

impl ContainerRuntime for PodmanCli {
    fn name(&self) -> &'static str {
        "cli"
    }

    fn run(
        &self,
        ctx: &Ctx,
        req: &Request<'_>,
        timeout: Option<Duration>,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        let podman = self.podman.display().to_string();
        let workdir = PathBuf::from(req.workdir);
        let probe = [
            podman.clone(),
            "image".to_owned(),
            "inspect".to_owned(),
            "--format".to_owned(),
            "{{.Digest}}".to_owned(),
            req.image.to_owned(),
        ];
        let digest = match ctx.proc()?.run(&probe, &workdir, PROBE_TIMEOUT) {
            Ok(out) if out.exit_code == 0 => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
            // Spec 231 R2: podman said the image is absent, or said nothing at all (spec 228). Any other words are the
            // runtime failing to answer, not an absent image.
            Ok(out) if probe_says_absent(&out.stderr) => {
                return Ok(Launch::Refused(Refusal::Image {
                    image: req.image.to_owned(),
                    reason: probe_reason(&out.stderr),
                }));
            }
            Ok(out) => {
                return Ok(Launch::Refused(Refusal::Runtime {
                    socket: podman,
                    status: None,
                    reason: probe_reason(&out.stderr),
                }));
            }
            Err(e) => return Ok(Launch::Refused(Refusal::Other(format!("{e}")))),
        };
        if digest.is_empty() {
            return Ok(Launch::Refused(Refusal::Image {
                image: req.image.to_owned(),
                reason: "image has no digest; pin it by digest".to_owned(),
            }));
        }
        let env_file = match env_file_path(self.runs_dir.as_deref()) {
            Ok(path) => path,
            Err(reason) => return Ok(Launch::Refused(Refusal::Other(reason))),
        };
        if let Err(e) = coder_container::write_env_file(&env_file, req.env) {
            return Ok(Launch::Refused(Refusal::Other(format!("env file: {e}"))));
        }
        let env_file_str = env_file.display().to_string();
        let run = ContainerRun {
            podman: &podman,
            image: req.image,
            workdir: req.workdir,
            env_file: &env_file_str,
            network: req.network,
            run_id: req.run_id,
            skills_dir: req.skills_dir,
        };
        let run_argv = coder_container::run_argv(&run, req.agent_argv);
        let result = wait_child(ctx, &run_argv, &workdir, &self.env, timeout, &mut |pid| {
            spawned(&Spawn {
                runtime: "cli",
                image: req.image.to_owned(),
                image_digest: digest.clone(),
                container: String::new(),
                pid,
                argv: run_argv.clone(),
                env_names: env_names(&self.env),
            });
        });
        // The env file holds the secrets: it goes on every path, once podman has had its read.
        let _ = std::fs::remove_file(&env_file);
        result
    }

    fn list_all(&self, ctx: &Ctx) -> Result<Vec<Listed>, PiiError> {
        let argv = [
            self.podman.display().to_string(),
            "ps".to_owned(),
            "--all".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ];
        let out = ctx.proc()?.run(&argv, Path::new("/"), PROBE_TIMEOUT)?;
        if out.exit_code != 0 {
            return Err(PiiError::Internal(format!(
                "coder runtime: podman ps: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let rows: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| PiiError::Internal(format!("coder runtime: podman ps: {e}")))?;
        Ok(rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let id = row.get("Id")?.as_str()?.to_owned();
                let labels = row
                    .get("Labels")?
                    .as_object()?
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect();
                Some(Listed { id, labels })
            })
            .collect())
    }

    fn force_remove(&self, ctx: &Ctx, id: &str) -> Result<bool, PiiError> {
        let argv = [
            self.podman.display().to_string(),
            "rm".to_owned(),
            "--force".to_owned(),
            id.to_owned(),
        ];
        let out = ctx.proc()?.run(&argv, Path::new("/"), PROBE_TIMEOUT)?;
        if out.exit_code == 0 {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if stderr.to_ascii_lowercase().contains("no such container") {
            return Ok(false);
        }
        Err(PiiError::Internal(format!(
            "coder runtime: podman rm {id}: {stderr}"
        )))
    }
}
