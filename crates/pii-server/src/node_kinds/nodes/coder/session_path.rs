//! Spec 271 R2: a pi coder node's default session file belongs to the node as well as the run.
//!
//! Two nodes of one run on one spec and workdir never resume one conversation; a loop, which re-runs one node, keeps its own file.

use std::path::Path;

use super::super::agent_worker::{Agent, WorkerSpec};

/// The node part of a session file name: every character outside `[A-Za-z0-9_.-]` becomes `_`, so an id can never add a
/// path separator. An id that is empty or only dots becomes `node`, so it can never name a parent directory.
fn node_part(node: &str) -> String {
    let safe: String = node
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.chars().all(|c| c == '.') {
        String::from("node")
    } else {
        safe
    }
}

/// The default session file of a pi coder node: `<workdir>/.pii-sessions/<spec-stem>-<run>-<node>.jsonl`.
#[must_use]
pub fn node_session_path(workdir: &str, spec_path: &str, run: &str, node: &str) -> String {
    let stem = Path::new(spec_path)
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or("worker");
    format!(
        "{workdir}/.pii-sessions/{stem}-{run}-{}.jsonl",
        node_part(node)
    )
}

/// A pi coder with no explicit `session`, whose cfg carries the `node-id` the build gave it, gets its node's session
/// file in its own workdir. Any other spec is returned as it stands.
#[must_use]
pub fn with_node_session(spec: WorkerSpec, cfg: &serde_json::Value) -> WorkerSpec {
    let node = cfg.get("node-id").and_then(serde_json::Value::as_str);
    let unpinned = spec.session.is_none() && spec.agent == Agent::Pi;
    match (node, spec.run.as_deref()) {
        (Some(node), Some(run)) if unpinned => {
            let session = node_session_path(&spec.workdir, &spec.spec, run, node);
            WorkerSpec {
                session: Some(session),
                ..spec
            }
        }
        _ => spec,
    }
}
