//! Spec 259: `judge` in spec mode.
//!
//! The criteria come from the spec's blob at a commit, never from the working tree, the run input or the agent: a
//! worker that edits its own spec gains nothing. Every criterion runs; a failure does not stop the rest, so the verdict
//! names all of them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_graph::expr::eval_or_literal;
use pii_graph::node::GraphNode;

/// How long one git read or criterion command may run before it is killed.
const CRITERION_TIMEOUT: Duration = Duration::from_mins(10);

/// The `judge` node in spec mode: its config is kept unresolved, since `spec`, `base` and `workdir` name run-input paths.
pub struct SpecJudgeNode {
    /// The judge's config as the graph wrote it, before its `$.` keys resolve against the run.
    pub cfg: serde_json::Value,
}

fn port(name: &str, value: serde_json::Value) -> Outcome {
    Outcome::Next {
        port: PortName::from(name),
        value,
    }
}

/// A commit id is exactly 40 lowercase hex digits; anything else is refused before git is asked.
fn is_commit_id(base: &str) -> bool {
    base.len() == 40
        && base
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The first backticked span of every `- [ ] ` or `- [x] ` line in the `## Exit Criteria` section.
fn parse_criteria(text: &str) -> Vec<String> {
    let mut in_section = false;
    let mut criteria = Vec::new();
    for line in text.lines() {
        if line.trim_end() == "## Exit Criteria" {
            in_section = true;
        } else if line.starts_with("## ") {
            in_section = false;
        } else if in_section && (line.starts_with("- [ ] ") || line.starts_with("- [x] ")) {
            // The span needs its closing backtick: `[_, command, _, ..]` matches only a closed span.
            let spans: Vec<&str> = line[6..].split('`').collect();
            if let [_, command, _, ..] = spans.as_slice() {
                criteria.push((*command).to_owned());
            }
        }
    }
    criteria
}

/// The value of `key` in the config, resolved against the run; a key the config lacks resolves to `Null`.
fn resolved(
    ctx: &serde_json::Value,
    cfg: &serde_json::Value,
    key: &str,
) -> Result<serde_json::Value, PiiError> {
    eval_or_literal(ctx, cfg.get(key).unwrap_or(&serde_json::Value::Null))
}

/// A `fail` outcome for a refusal that happens before any criterion runs.
fn refusal(spec: &serde_json::Value, base: &serde_json::Value, reason: &str) -> Outcome {
    port(
        "fail",
        serde_json::json!({ "verdict": "failed", "spec": spec, "base": base, "reason": reason }),
    )
}

/// The exit code of every command, in order. A failure does not stop the rest.
fn run_all(ctx: &Ctx, workdir: &Path, commands: &[String]) -> Result<Vec<i32>, PiiError> {
    commands
        .iter()
        .map(|command| -> Result<i32, PiiError> {
            let argv = ["/bin/sh".to_owned(), "-c".to_owned(), command.clone()];
            Ok(ctx
                .proc()?
                .run(&argv, workdir, CRITERION_TIMEOUT)?
                .exit_code)
        })
        .collect()
}

impl GraphNode for SpecJudgeNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let scope = serde_json::json!({
            "input": input.input,
            "payload": input.payload,
            "nodes": input.nodes,
        });
        let spec = resolved(&scope, &self.cfg, "spec")?;
        let base = resolved(&scope, &self.cfg, "base")?;
        let workdir = resolved(&scope, &self.cfg, "workdir")?;
        let base_text = base.as_str().unwrap_or_default();
        if !is_commit_id(base_text) {
            let got = base
                .as_str()
                .map_or_else(|| base.to_string(), str::to_owned);
            let reason = format!("judge: base must be a 40-hex commit id, got '{got}'");
            return Ok(refusal(&spec, &base, &reason));
        }
        let Some(workdir) = workdir.as_str().map(PathBuf::from) else {
            return Ok(refusal(&spec, &base, "judge: config needs `workdir`"));
        };
        let spec_text = spec.as_str().unwrap_or_default();
        // The blob is read by a fixed script; `spec` and `base` are its positional arguments, never part of the text.
        let argv = [
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            r#"git show "$1:$2""#.to_owned(),
            "sh".to_owned(),
            base_text.to_owned(),
            spec_text.to_owned(),
        ];
        let read = ctx.proc()?.run(&argv, &workdir, CRITERION_TIMEOUT)?;
        if read.exit_code != 0 {
            let stderr = String::from_utf8_lossy(&read.stderr);
            let first = stderr.lines().next().unwrap_or_default();
            let reason = format!("judge: cannot read {spec_text} at {base_text}: {first}");
            return Ok(refusal(&spec, &base, &reason));
        }
        let criteria = parse_criteria(&String::from_utf8_lossy(&read.stdout));
        if criteria.is_empty() {
            let reason = format!("judge: {spec_text} has no Exit Criteria");
            return Ok(refusal(&spec, &base, &reason));
        }
        let rcs = run_all(ctx, &workdir, &criteria)?;
        let results: Vec<serde_json::Value> = criteria
            .iter()
            .zip(&rcs)
            .map(|(command, rc)| serde_json::json!({ "criterion": command, "rc": rc }))
            .collect();
        let failing: Vec<(&String, &i32)> = criteria
            .iter()
            .zip(&rcs)
            .filter(|(_, rc)| **rc != 0)
            .collect();
        let mut verdict = serde_json::json!({
            "verdict": if failing.is_empty() { "passed" } else { "failed" },
            "spec": spec,
            "base": base,
            "criteria": criteria.len(),
            "passed": criteria.len() - failing.len(),
            "results": results,
        });
        let Some((command, rc)) = failing.first() else {
            return Ok(port("ok", verdict));
        };
        let reason = format!("criterion failed: {command} (rc {rc})");
        let names: Vec<&str> = failing.iter().map(|(c, _)| c.as_str()).collect();
        if let Some(map) = verdict.as_object_mut() {
            map.insert("reason".to_owned(), reason.into());
            map.insert("failing".to_owned(), names.into());
        }
        Ok(port("fail", verdict))
    }
}
