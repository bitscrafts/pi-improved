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
        validate_signature(sig)?;
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
        if !is_contained(&dir, &wt) {
            return Err(PiiError::Invalid(format!(
                "worktree: signature escapes heal-worktrees dir: {sig:?}"
            )));
        }
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

/// A signature must be a lowercase-or-uppercase hex string of at least 8
/// characters (blake3 hex prefixes). Anything else — path separators,
/// absolute paths, non-hex characters — is rejected before it can reach
/// `dir.join(sig)` or a branch name.
fn validate_signature(sig: &str) -> Result<(), PiiError> {
    const MIN_SIG_LEN: usize = 8;
    if sig.len() < MIN_SIG_LEN {
        return Err(PiiError::Invalid(format!(
            "worktree: signature must be at least {MIN_SIG_LEN} hex chars, got {sig:?}"
        )));
    }
    if !sig.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(PiiError::Invalid(format!(
            "worktree: signature must be hex, got {sig:?}"
        )));
    }
    Ok(())
}

/// Defensive containment check, independent of the signature regex: returns
/// `false` unless `candidate` is a strict child of `dir`.
fn is_contained(dir: &Path, candidate: &Path) -> bool {
    candidate.strip_prefix(dir).is_ok_and(|rel| {
        !rel.as_os_str().is_empty()
            && !rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
    })
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::path::{Path, PathBuf};

    use pii_abi::PiiError;
    use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
    use serde_json::json;

    use super::WorktreeTool;
    use crate::Tool;

    fn tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A root context granting `PROC` for `/bin/sh` (all worktree git calls
    /// go through `/bin/sh -c`).
    fn proc_ctx() -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::PROC;
        caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
        Ctx::root(caps, Budget::unlimited())
    }

    /// A temp dir that is a real git repo with one commit on `HEAD`
    /// (`worktree add ... HEAD` needs it).
    ///
    /// Uses `std::process::Command` directly rather than `Ctx::proc()`:
    /// this is one-off test-fixture setup (not the `WorktreeTool` under
    /// test), so routing it through the capability-gated `proc()` path
    /// would just add ceremony without testing anything — the disallowed-
    /// methods lint exists to keep PRODUCTION spawn paths funneled
    /// through `Ctx::proc()`, not test scaffolding.
    #[allow(clippy::disallowed_methods)]
    fn git_repo(prefix: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let run = |cmd: &str| {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(&canon)
                .output()
                .expect("spawn git");
            assert!(
                out.status.success(),
                "setup `{cmd}` failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run("git init -q -b main .");
        run("git config user.email t@t && git config user.name t");
        std::fs::write(canon.join("seed.txt"), "seed\n").unwrap();
        run("git add seed.txt && git commit -q -m seed");
        (dir, canon)
    }

    /// Best-effort cleanup between tests sharing a repo layout; see
    /// `git_repo`'s rationale for the direct `Command` use.
    #[allow(clippy::disallowed_methods)]
    fn cleanup_worktrees(canon: &PathBuf) {
        let _ = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("git worktree prune")
            .current_dir(canon)
            .output();
    }

    #[test]
    fn worktree_requires_signature_string() {
        let (_d, canon) = git_repo("wt-miss-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool { workspace: canon };
        let err = tool.call(&ctx, &json!({})).unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
    }

    #[test]
    fn worktree_provisions_and_reports_base_commit() {
        let (_d, canon) = git_repo("wt-add-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        let res = tool
            .call(&ctx, &json!({"signature": "a11add7e5f60"}))
            .expect("worktree add must succeed in a real repo");
        let wt = res["worktree"].as_str().unwrap();
        assert!(wt.ends_with("_tmp/heal-worktrees/a11add7e5f60"), "got {wt}");
        assert_eq!(res["branch"].as_str().unwrap(), "heal/a11add7e5f60");
        let base = res["base_commit"].as_str().unwrap();
        assert_eq!(base.len(), 40, "full hex sha expected, got {base}");
        // The worktree really exists on disk.
        assert!(Path::new(wt).exists());
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_rejects_duplicate_signature() {
        let (_d, canon) = git_repo("wt-dup-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        tool.call(&ctx, &json!({"signature": "b0bd1c35"}))
            .expect("first provision must succeed");
        let err = tool
            .call(&ctx, &json!({"signature": "b0bd1c35"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_denied_without_proc_cap() {
        // A context without `PROC` can never run git → Denied before anything.
        let dir = tempfile::Builder::new()
            .prefix("wt-nocap-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let tool = WorktreeTool { workspace: canon };
        let err = tool
            .call(&ctx, &json!({"signature": "0ca47a55"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Denied(_)), "got {err:?}");
    }

    #[test]
    fn worktree_rejects_path_traversal_signature() {
        let (_d, canon) = git_repo("wt-trav-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        let err = tool
            .call(&ctx, &json!({"signature": "../../../etc"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        assert!(
            !canon.join("etc").exists(),
            "no directory may be created outside heal-worktrees/"
        );
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_rejects_absolute_path_signature() {
        let (_d, canon) = git_repo("wt-abs-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        let err = tool
            .call(&ctx, &json!({"signature": "/tmp/evil"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        assert!(!PathBuf::from("/tmp/evil").exists(), "got {err:?}");
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_rejects_non_hex_signature() {
        let (_d, canon) = git_repo("wt-nonhex-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        let err = tool
            .call(&ctx, &json!({"signature": "not-hex-chars!"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_rejects_too_short_signature() {
        let (_d, canon) = git_repo("wt-short-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        let err = tool.call(&ctx, &json!({"signature": "abc"})).unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_accepts_valid_hex_signature() {
        let (_d, canon) = git_repo("wt-hex-");
        cleanup_worktrees(&canon);
        let ctx = proc_ctx();
        let tool = WorktreeTool {
            workspace: canon.clone(),
        };
        let res = tool
            .call(&ctx, &json!({"signature": "a1b2c3d4e5f6"}))
            .expect("valid hex signature must provision");
        let wt = res["worktree"].as_str().unwrap();
        assert!(wt.ends_with("_tmp/heal-worktrees/a1b2c3d4e5f6"), "got {wt}");
        assert_eq!(res["branch"].as_str().unwrap(), "heal/a1b2c3d4e5f6");
        assert!(Path::new(wt).exists());
        cleanup_worktrees(&canon);
    }

    #[test]
    fn worktree_containment_check_is_defensive() {
        let dir = PathBuf::from("/tmp/containment-root");
        // Direct children are contained.
        assert!(super::is_contained(&dir, &dir.join("a1b2c3d4")));
        // Escapes are not.
        assert!(!super::is_contained(&dir, &dir.join("..").join("evil")));
        assert!(!super::is_contained(&dir, Path::new("/tmp/evil")));
        // The dir itself is not a child.
        assert!(!super::is_contained(&dir, &dir));
        // Prefix tricks do not fool strip_prefix.
        assert!(!super::is_contained(
            &dir,
            &PathBuf::from("/tmp/containment-root-evil/x")
        ));
    }
}
