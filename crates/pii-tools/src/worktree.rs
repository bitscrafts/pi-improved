//! `worktree` tool (spec 32 R2): provisions a disposable git worktree for a
//! heal attempt, returning `{worktree, branch, base_commit}`.
//!
//! All process execution goes through `Ctx::proc()` (the crate forbids
//! `std::process::Command`); git is invoked via `/bin/sh -c` (the only
//! `proc_argv0` a run grants).

use std::path::Path;
use std::time::Duration;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_model::ToolSpec;
use serde_json::{Value, json};

use crate::Tool;

const MAX_WORKTREES: usize = 50;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// `worktree { signature, workspace }` — provisions a worktree under
/// `<workspace>/_tmp/heal-worktrees/<signature>` on branch `heal/<signature>`.
pub struct WorktreeTool {
    /// Workspace the tool is bound to (worktrees live under its `_tmp/`).
    pub workspace: std::path::PathBuf,
}

impl Tool for WorktreeTool {
    fn name(&self) -> &'static str {
        "worktree"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("worktree"),
            description: String::from(
                "Provision a disposable git worktree for a heal attempt; returns {worktree, branch, base_commit}",
            ),
            schema: json!({
                "type": "object",
                "properties": {
                    "signature": { "type": "string", "description": "heal signature (blake3 hex)" }
                },
                "required": ["signature"]
            }),
        }
    }

    fn timeout(&self) -> Option<u64> {
        // git worktree add can be slow on a cold checkout.
        Some(60_000)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let sig = args
            .get("signature")
            .and_then(Value::as_str)
            .ok_or_else(|| PiiError::Invalid("worktree requires 'signature' string".into()))?;
        let dir = self.workspace.join("_tmp").join("heal-worktrees");
        std::fs::create_dir_all(&dir)
            .map_err(|e| PiiError::Internal(format!("worktree mkdir: {e}")))?;
        if dir.read_dir().is_ok_and(|d| d.count() >= MAX_WORKTREES) {
            return Err(PiiError::Invalid(format!(
                "worktree: >= {MAX_WORKTREES} worktrees; run heal-prune first"
            )));
        }
        if dir_size(&dir)? > MAX_BYTES {
            return Err(PiiError::Invalid(String::from(
                "worktree: worktrees > 2 GiB; run heal-prune first",
            )));
        }
        let wt = dir.join(sig);
        if wt.exists() {
            return Err(PiiError::Invalid(format!(
                "worktree: {sig} already provisioned at {}",
                wt.display()
            )));
        }
        let branch = format!("heal/{sig}");
        let wt_str = wt.to_string_lossy().into_owned();
        // git worktree add -b heal/<sig> <path> HEAD
        let add = format!(
            "git worktree add -b {} {} HEAD",
            shell_quote(&branch),
            shell_quote(&wt_str)
        );
        let out = ctx.proc()?.run(
            &[String::from("/bin/sh"), String::from("-c"), add],
            &self.workspace,
            Duration::from_mins(1),
        )?;
        if out.exit_code != 0 {
            return Err(PiiError::Internal(format!(
                "worktree add failed (exit {}): {}",
                out.exit_code,
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        let base = ctx.proc()?.run(
            &[
                String::from("/bin/sh"),
                String::from("-c"),
                String::from("git rev-parse HEAD"),
            ],
            &self.workspace,
            Duration::from_secs(30),
        )?;
        let base_commit = String::from_utf8_lossy(&base.stdout).trim().to_owned();
        Ok(json!({
            "worktree": wt_str,
            "branch": branch,
            "base_commit": base_commit,
        }))
    }
}

/// Minimal single-quote shell escaping for a path/branch (no embedded `'`).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn dir_size(dir: &Path) -> Result<u64, PiiError> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir).map_err(|e| PiiError::Internal(e.to_string()))? {
        let entry = entry.map_err(|e| PiiError::Internal(e.to_string()))?;
        let md = entry
            .metadata()
            .map_err(|e| PiiError::Internal(e.to_string()))?;
        if md.is_dir() {
            total = total.saturating_add(dir_size(&entry.path())?);
        } else {
            total = total.saturating_add(md.len());
        }
    }
    Ok(total)
}
