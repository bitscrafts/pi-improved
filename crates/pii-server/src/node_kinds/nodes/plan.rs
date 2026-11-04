//! `plan` node kind (spec 363 R1, R5): the planner role turns a finding into one new spec under `specs/`.
//!
//! The planner is a model call with no tools. It returns the spec's text, and the node writes that text to
//! `specs/<NN>-<slug>.md`, where `NN` is the next free number. The node chooses the path, refuses a path that exists,
//! and writes only through the run's `fs_write` scope, so the planner never writes a file itself. The planner does not
//! implement the spec it wrote; `enqueue` admits it, and a separate run judges the work.

use std::path::Path;
use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};
use pii_model::{ChatMessage, ChatRequest, ModelProvider, Role};

/// The longest slug a spec file name takes from its finding.
const SLUG_MAX: usize = 60;

/// The longest title a spec row carries, taken from the finding's first line.
const TITLE_MAX: usize = 120;

/// The planner's instructions: the house shape the lane's validators read, and the rule that it writes no file.
const PLANNER_SYSTEM: &str = "You write one spec for the pii repository. Reply with the spec document only: no preamble, \
no code fence, and no file writes. Do not implement the spec.";

/// The house shape: `## Files` with one backticked path per bullet, and `## Exit Criteria` with one command per line.
const HOUSE_SHAPE: &str = "Use exactly these headings, in this order:\n\
# Spec: <title>\n\
## Requirements (R1., R2., ... each one checkable)\n\
## TDD contract (a table: file | test | assertion)\n\
## Exit Criteria (one line each, exactly: - [ ] `command`)\n\
## Files (one line each, exactly: - `path`)\n\
Every Exit Criteria line is one shell command in backticks. Every Files line names one file path in backticks.";

/// Factory for `plan`: the planner model is `provider`, and each node names its model in config.
pub struct PlanFactory {
    /// The provider the planner's model is called through.
    pub provider: Arc<dyn ModelProvider>,
}

/// A built `plan` node. Its config is resolved at run time, so a `$.` expression in it reads the run's blackboard.
struct PlanNode {
    provider: Arc<dyn ModelProvider>,
    cfg: serde_json::Value,
}

/// The config's text under `key`, with a `$.` expression resolved against the node's input.
pub(crate) fn resolved_text(
    cfg: &serde_json::Value,
    input: &NodeInput,
    key: &str,
) -> Option<String> {
    let ctx = serde_json::json!({
        "input": input.input,
        "payload": input.payload,
        "nodes": input.nodes,
    });
    let value = eval_or_literal(&ctx, cfg.get(key)?).ok()?;
    value.as_str().map(str::to_owned)
}

fn port(name: &str, value: serde_json::Value) -> Outcome {
    Outcome::Next {
        port: PortName::from(name),
        value,
    }
}

/// The next free spec number: one past the largest leading number among `specs/` file names, or 1 when none has one.
fn next_number(specs: &Path) -> Result<u64, String> {
    let entries = match std::fs::read_dir(specs) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(1),
        Err(e) => return Err(format!("read {}: {e}", specs.display())),
    };
    let mut highest = 0;
    for entry in entries {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        let leading = name.to_string_lossy();
        if let Some(n) = leading
            .split('-')
            .next()
            .and_then(|s| s.parse::<u64>().ok())
        {
            highest = highest.max(n);
        }
    }
    Ok(highest + 1)
}

/// A file-name slug of the finding: lowercase words joined by `-`, at most [`SLUG_MAX`] characters.
fn slug(finding: &str) -> String {
    let mut out = String::new();
    for c in finding.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let cut: String = out.chars().take(SLUG_MAX).collect();
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() {
        "finding".to_owned()
    } else {
        cut.to_owned()
    }
}

/// The planner's reply without a surrounding code fence, trimmed, ending in one newline.
fn spec_body(reply: &str) -> String {
    let mut text = reply.trim();
    if let Some(inner) = text.strip_prefix("```") {
        let inner = inner.split_once('\n').map_or("", |(_, rest)| rest);
        text = inner.trim_end().strip_suffix("```").unwrap_or(inner).trim();
    }
    format!("{text}\n")
}

impl PlanNode {
    fn prompt(finding: &str, root: &str) -> String {
        format!(
            "Finding: {finding}\nRepository: {root}\n\nWrite the spec that fixes this finding.\n{HOUSE_SHAPE}"
        )
    }

    fn refuse(reason: impl Into<String>) -> Outcome {
        port("err", serde_json::json!({ "reason": reason.into() }))
    }
}

impl GraphNode for PlanNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let finding = resolved_text(&self.cfg, input, "finding").unwrap_or_default();
        if finding.trim().is_empty() {
            return Ok(Self::refuse("plan needs a finding"));
        }
        let Some(root) = resolved_text(&self.cfg, input, "workdir") else {
            return Ok(Self::refuse("plan needs a workdir"));
        };
        let Some(model) = resolved_text(&self.cfg, input, "model") else {
            return Ok(Self::refuse("plan needs a planner model"));
        };
        let number = match next_number(&Path::new(&root).join("specs")) {
            Ok(number) => number,
            Err(reason) => return Ok(Self::refuse(reason)),
        };
        let spec = format!("specs/{number}-{}.md", slug(&finding));
        let request = ChatRequest {
            model,
            messages: vec![
                ChatMessage::new(Role::System, PLANNER_SYSTEM),
                ChatMessage::new(Role::User, Self::prompt(&finding, &root)),
            ],
            tools: Vec::new(),
            max_tokens: 8192,
            temperature: 0.0,
            num_ctx: 0,
        };
        let reply = match self.provider.chat(ctx, &request) {
            Ok(reply) => reply,
            Err(e) => return Ok(Self::refuse(format!("planner call failed: {e}"))),
        };
        if reply.text.trim().is_empty() {
            return Ok(Self::refuse("planner returned no text"));
        }
        let body = spec_body(&reply.text);
        // The one write: the path is `specs/` by construction, the run's fs_write scope bounds it, and an existing file
        // is never replaced.
        let target = Path::new(&root).join(&spec);
        if target.exists() {
            return Ok(Self::refuse(format!("{spec} already exists")));
        }
        ctx.fs()?.write(&target, body.as_bytes())?;
        let title: String = finding
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(TITLE_MAX)
            .collect();
        Ok(port(
            "ok",
            serde_json::json!({ "spec": spec, "number": number, "title": title }),
        ))
    }
}

impl NodeFactory for PlanFactory {
    fn kind(&self) -> &'static str {
        "plan"
    }

    fn is_effect(&self) -> bool {
        true
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::FS_WRITE
    }

    fn ports(&self, _cfg: &serde_json::Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &serde_json::Value) -> Result<Box<dyn GraphNode>, PiiError> {
        Ok(Box::new(PlanNode {
            provider: Arc::clone(&self.provider),
            cfg: cfg.clone(),
        }))
    }
}
