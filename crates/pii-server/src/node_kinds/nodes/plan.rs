//! `plan` node kind (spec 363 R1, R5): the planner role turns a finding into one new spec under `specs/`.
//!
//! The planner is a model call with no tools. It returns the spec's text, and the node writes that text to
//! `specs/<NN>-<slug>.md`, where `NN` is the next free number. The node chooses the path, refuses a path that exists,
//! and writes only through the run's `fs_write` scope, so the planner never writes a file itself. The planner does not
//! implement the spec it wrote; `enqueue` admits it, and a separate run judges the work.

use std::path::{Component, Path, PathBuf};
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
        write_spec(ctx, &finding, &root, &reply.text, None)
    }
}

/// Spec 414 R1-R3: the directory a spec is written into. Without a workspace (`plan`) `root` is used as given. With one
/// (`spec.write`), a relative `root` is inside the workspace, and a `root` that leaves it (`..`, or a symlink out) is
/// refused, naming the path. An absolute `root` is used as given; the run's `fs_write` scope still bounds the write.
fn spec_root(root: &str, workspace: Option<&Path>) -> Result<PathBuf, String> {
    let Some(workspace) = workspace else {
        return Ok(PathBuf::from(root));
    };
    let rel = Path::new(root);
    if rel.is_absolute() {
        return Ok(rel.to_path_buf());
    }
    let leaves = || format!("workdir {root} leaves the workspace");
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(leaves());
    }
    let joined = workspace.join(rel);
    let resolved = match std::fs::canonicalize(&joined) {
        Ok(resolved) => resolved,
        // Not there yet, and not a link that could point outside: nothing can escape through it.
        Err(_) if joined.symlink_metadata().is_err() => return Ok(joined),
        Err(_) => return Err(leaves()),
    };
    let inside = std::fs::canonicalize(workspace)
        .map_err(|e| format!("workspace {}: {e}", workspace.display()))?;
    if resolved.starts_with(&inside) {
        Ok(joined)
    } else {
        Err(leaves())
    }
}

/// Spec 392 R3: the half of a planner that is not a model call. Writes the reply to the next free `specs/<NN>-<slug>.md`.
fn write_spec(
    ctx: &Ctx,
    finding: &str,
    root: &str,
    reply: &str,
    workspace: Option<&Path>,
) -> Result<Outcome, PiiError> {
    if reply.trim().is_empty() {
        return Ok(PlanNode::refuse("planner returned no text"));
    }
    let root = match spec_root(root, workspace) {
        Ok(root) => root,
        Err(reason) => return Ok(PlanNode::refuse(reason)),
    };
    let number = match next_number(&root.join("specs")) {
        Ok(number) => number,
        Err(reason) => return Ok(PlanNode::refuse(reason)),
    };
    let spec = format!("specs/{number}-{}.md", slug(finding));
    let body = spec_body(reply);
    // The one write: the path is `specs/` by construction, the run's fs_write scope bounds it, and an existing file
    // is never replaced.
    let target = root.join(&spec);
    if target.exists() {
        return Ok(PlanNode::refuse(format!("{spec} already exists")));
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

/// `ledger` op `spec.write` (spec 392 R3): an `agent` node drafts the spec, and this node writes the draft in `text`.
/// Spec 414: with a `workspace` (the daemon's), a relative `workdir` is inside it; without one, `workdir` is as given.
pub(super) struct SpecWriteNode {
    pub(super) cfg: serde_json::Value,
    pub(super) workspace: Option<PathBuf>,
}

impl GraphNode for SpecWriteNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let finding = resolved_text(&self.cfg, input, "finding").unwrap_or_default();
        if finding.trim().is_empty() {
            return Ok(PlanNode::refuse("plan needs a finding"));
        }
        let Some(root) = resolved_text(&self.cfg, input, "workdir") else {
            return Ok(PlanNode::refuse("plan needs a workdir"));
        };
        let text = resolved_text(&self.cfg, input, "text").unwrap_or_default();
        write_spec(ctx, &finding, &root, &text, self.workspace.as_deref())
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
