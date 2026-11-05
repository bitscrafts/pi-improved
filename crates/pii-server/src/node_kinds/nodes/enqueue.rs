//! `enqueue` node kind (spec 363 R3): admits a spec `plan` wrote to the development queue, as one row.
//!
//! The row's criteria are the spec's `## Exit Criteria` and its files the spec's `## Files`, read with the same rules the
//! lane's validators use (`row-verify.py` and `row-paths.py`). A spec that would fail either validator is refused with
//! the reason, and nothing is written to the queue.

use std::path::Path;
use std::sync::Arc;

use pii_abi::{NodeInput, Outcome, PiiError, PortName};
use pii_core::Ctx;
use pii_graph::node::GraphNode;
use pii_store::queue::{QueueStore, WorkItem};

use super::plan::resolved_text;

/// `queue.put` (spec 391, was `enqueue`): the queue store is the one the `select` node claims from.
/// Its config is resolved at run time, as `plan`'s is.
pub(super) struct EnqueueNode {
    pub(super) store: Arc<QueueStore>,
    pub(super) cfg: serde_json::Value,
}

/// The `## Files` entries in order: the first backticked span of each bullet under the heading. Mirrors `row-paths.py`.
fn declared_files(text: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            inside = line.trim() == "## Files";
        } else if let Some(entry) = (inside && line.matches('`').count() >= 2)
            .then(|| line.split('`').nth(1).map(str::trim))
            .flatten()
            .filter(|e| !e.is_empty())
        {
            entries.push(entry.to_owned());
        }
    }
    entries
}

/// The `## Exit Criteria` commands in order. Mirrors `row-verify.py`: every line under the heading must be one
/// criterion, a checkbox line with one backticked command, and the error names the first line that is not.
fn criteria(text: &str) -> Result<Vec<String>, String> {
    let mut commands = Vec::new();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("## ") {
            inside = line == "## Exit Criteria";
        } else if inside && line.starts_with("- [") {
            let command = line
                .strip_prefix("- [ ] `")
                .and_then(|rest| rest.strip_suffix('`'))
                .map(str::trim)
                .filter(|c| !c.is_empty() && !c.contains('`'));
            match command {
                Some(command) => commands.push(command.to_owned()),
                None => return Err(format!("NOT-A-CRITERION {line}")),
            }
        }
    }
    Ok(commands)
}

/// The refusal for a spec that fails the house shape, or the reason, as the validators would print it.
fn shape_refusal(
    spec: &str,
    text: &str,
    workdir: &Path,
) -> Result<(Vec<String>, Vec<String>), String> {
    let files = declared_files(text);
    if files.is_empty() {
        return Err(format!(
            "NO-FILES {spec}: no `## Files` heading with a backticked path per bullet"
        ));
    }
    if let Some(dir) = files
        .iter()
        .find(|f| f.ends_with('/') || workdir.join(f).is_dir())
    {
        return Err(format!("NOT-A-FILE {dir}"));
    }
    let commands = criteria(text)?;
    if commands.is_empty() {
        return Err(format!(
            "NO-CRITERIA {spec}: no `## Exit Criteria` heading with `- [ ] `command`` lines"
        ));
    }
    if let Some(split) = commands.iter().find(|c| c.contains(';')) {
        return Err(format!(
            "SPLIT {split}: join the commands with && inside one criterion"
        ));
    }
    Ok((commands, files))
}

fn port(name: &str, value: serde_json::Value) -> Outcome {
    Outcome::Next {
        port: PortName::from(name),
        value,
    }
}

impl GraphNode for EnqueueNode {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let refuse = |reason: String| port("err", serde_json::json!({ "reason": reason }));
        let Some(spec) = resolved_text(&self.cfg, input, "spec").filter(|s| !s.is_empty()) else {
            return Ok(refuse("enqueue needs a spec path".to_owned()));
        };
        let Some(workdir) = resolved_text(&self.cfg, input, "workdir") else {
            return Ok(refuse("enqueue needs a workdir".to_owned()));
        };
        let workdir = Path::new(&workdir);
        let bytes = match ctx.fs()?.read(&workdir.join(&spec)) {
            Ok(bytes) => bytes,
            Err(e) => return Ok(refuse(format!("spec {spec} cannot be read: {e}"))),
        };
        let text = String::from_utf8_lossy(&bytes);
        let (commands, files) = match shape_refusal(&spec, &text, workdir) {
            Ok(shape) => shape,
            Err(reason) => return Ok(refuse(reason)),
        };
        let id = Path::new(&spec)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.store.get_item(&id)?.is_some() {
            return Ok(refuse(format!("{id} is already in the queue")));
        }
        let title = resolved_text(&self.cfg, input, "title")
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| id.clone());
        let mut item = WorkItem::new(&id, &[]);
        item.spec.clone_from(&spec);
        item.title = title;
        item.criteria.clone_from(&commands);
        item.touches.clone_from(&files);
        self.store.put_item(&item)?;
        Ok(port(
            "ok",
            serde_json::json!({ "id": id, "spec": spec, "criteria": commands, "touches": files }),
        ))
    }
}
