//! The `GraphNode` the coder node runs as.

use std::path::PathBuf;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_graph::node::GraphNode;

use super::super::agent_worker::{CriterionRun, Verdict, WorkerSpec, decide, plan};
use super::super::coder::CRITERION_TIMEOUT;
use super::super::coder_container::CONTAINER_WORKDIR;
use super::super::coder_runtime::{Launch, Refusal};
use super::super::coder_skills::{self, SkillRefusal};
use pii_observe::{Event, Level, names};

use super::CoderNode;
use super::helpers::{
    deferred_verdict, refuse_planned, stop_reason, text, texts, with_selected_spec,
};

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
        let scope_root = input
            .input
            .get("scope_root")
            .and_then(serde_json::Value::as_str);
        let checks = match self.resolve_criteria(cfg, scope_root) {
            Ok(checks) => checks,
            Err(name) => return Ok(self.refuse_criterion(&name, &agent, &model)),
        };
        let env = self.pass_env(&texts(cfg, "env_pass"));
        let image = text(cfg, "container_image").filter(|i| !i.is_empty());
        // Spec 230: the runtime is named by graph data; an unknown name refuses before anything spawns.
        let runtime = text(cfg, "container_runtime").unwrap_or_else(|| String::from("cli"));
        if image.is_some() && runtime != "cli" && runtime != "socket" {
            let reason = format!("container_runtime '{runtime}' is not cli or socket");
            return Ok(self.refuse(ctx, &agent, &model, &reason));
        }
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
            self.launch_in_container(ctx, image, &spec, env, &dispatch.argv, &skills, &runtime)?
        } else {
            self.log.event(
                &Event::new("coder", Level::Warn, names::CODER_HOST_SPAWN_DEPRECATED)
                    .field("agent", agent.clone()),
            );
            self.launch_on_host(ctx, &dispatch.argv, &spec, env)?
        };
        let (code, pid, dur_ms, stop) = match launch {
            Launch::Exited { code, pid, dur_ms } => (code, pid, dur_ms, stop_reason(code)),
            // A refusal is journalled before it routes (spec 217 R5): no pid, and no `coder.exited`. No spawn ran, so an
            // image node's digest slot says what the probe found (spec 223 R3, 231 R2): `missing` when the runtime said
            // the image is absent, `probe-failed` when it could not answer.
            Launch::Refused(refusal) => {
                self.journal_refusal(&refusal);
                let digest = image.as_ref().map(|_| match refusal {
                    Refusal::Image { .. } => "missing",
                    Refusal::Runtime { .. } | Refusal::Other(_) => "probe-failed",
                });
                return Ok(self.refuse_with(ctx, &agent, &model, &refusal.reason(), digest));
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
