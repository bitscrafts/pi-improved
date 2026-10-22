//! The `GraphNode` the coder node runs as.

use std::path::PathBuf;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_graph::node::GraphNode;

use super::super::agent_worker::{
    CriterionRun, Verdict, WorkerSpec, decide, plan, reported_routes,
};
use super::super::coder::CRITERION_TIMEOUT;
use super::super::coder::redact;
use super::super::coder::report::RunReport;
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
            (Ok(spec), Some(brief)) if !spec.spec.is_empty() && super::brief_is_usable(&brief) => {
                (spec, brief)
            }
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
            self.launch_on_host(ctx, &dispatch.argv, &spec, env, super::agent_timeout(cfg))?
        };
        let (code, pid, dur_ms, stop, output, stderr) = match launch {
            Launch::Exited {
                code,
                pid,
                dur_ms,
                output,
                stderr,
            } => (code, pid, dur_ms, stop_reason(code), output, stderr),
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
        // The agent's output is journalled and stored: scrub the credentials pii passed it, and any token-shaped
        // string it printed itself, before anything downstream sees the text.
        let secrets = redact::credential_values(&self.pass_env(&texts(cfg, "env_pass")));
        let output = redact::redact(&output, &secrets);
        let stderr = redact::redact(&stderr, &secrets);

        // The criteria decide. The first failing criterion stops the run and names itself in the verdict.
        // Its output tail is kept for the journal: a bare rc says the check failed, not why.
        let mut runs = Vec::new();
        let mut failure_tail = String::new();
        for (command, argv) in checks {
            let out = ctx.proc()?.run(&argv, &workdir, CRITERION_TIMEOUT)?;
            let rc = out.exit_code;
            runs.push(CriterionRun { command, rc });
            if rc != 0 {
                failure_tail = redact::redact(&output_tail(&out.stdout, &out.stderr), &secrets);
                break;
            }
        }
        let report = RunReport {
            runs: &runs,
            failure_tail: &failure_tail,
            routes: &reported_routes(&output),
            agent_output: &output,
            agent_stderr: &stderr,
        };
        self.emit_exited(ctx, pid, code, dur_ms, stop, &report);

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

/// The last part of a failing criterion's output, for the journal. A bare exit code does not say why the check
/// failed. Bounded to the tail so a long build log cannot flood the journal.
const OUTPUT_TAIL_BYTES: usize = 2000;

fn output_tail(stdout: &[u8], stderr: &[u8]) -> String {
    // Keep the HEAD as well as the tail: a cargo failure names its cause near the start, and the tail
    // alone is often a successful compile (the rlib list) that hides it.
    let mut text = String::from_utf8_lossy(stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(stdout));
    if text.len() <= OUTPUT_TAIL_BYTES {
        return text;
    }
    let half = OUTPUT_TAIL_BYTES / 2;
    let head_end = char_floor(&text, half);
    let tail_start = char_floor(&text, text.len() - half);
    format!("{} ... {}", &text[..head_end], &text[tail_start..])
}

/// The largest char boundary at or below `i`.
fn char_floor(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
