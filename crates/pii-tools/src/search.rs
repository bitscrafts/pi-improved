//! Search tools (spec 13 F4): `grep` and `glob`, read-only and
//! workspace-scoped. File reads go through `Ctx::fs()`; walking is confined
//! to the workspace root.

use std::path::{Path, PathBuf};

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_model::ToolSpec;
use serde_json::{Value, json};

use crate::Tool;

/// Caps combined output at 64 KiB with a marker (mirrors `bash`'s cap).
const MAX_OUTPUT: usize = 64 * 1024;
const TRUNCATION_MARKER: &str = "\n[...truncated...]";

fn require<'a>(args: &'a Value, key: &str) -> Result<&'a str, PiiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| PiiError::Invalid(format!("{key} must be a string")))
}

/// Resolves a `cwd` argument against the workspace; rejects anything outside.
fn resolve_cwd(workspace: &Path, cwd: Option<&str>) -> Result<PathBuf, PiiError> {
    let base = match cwd {
        Some(c) => {
            let canon =
                dunce::canonicalize(c).map_err(|e| PiiError::Invalid(format!("cwd {c}: {e}")))?;
            if !canon.starts_with(workspace) {
                return Err(PiiError::Denied(format!(
                    "cwd {c} is outside the workspace"
                )));
            }
            canon
        }
        None => workspace.to_path_buf(),
    };
    Ok(base)
}

/// Recursively walks `root`, yielding every file path (not dirs), skipping
/// `.git` and `target` (build artifacts are noise for search).
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".git" || name == "target" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// Minimal glob matcher: `*` within a segment, `**` across segments.
/// No external crate (spec 13 N1); `?` unsupported to stay small.
fn glob_match(pattern: &str, path: &str) -> bool {
    let patterns: Vec<&str> = pattern.split('/').collect();
    let segments: Vec<&str> = path.split('/').collect();
    match_segs(&patterns, &segments)
}

fn match_segs(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            // `**` matches zero or more path segments.
            (0..=path.len()).any(|skip| {
                match_segs(pat.get(1..).unwrap_or(&[]), path.get(skip..).unwrap_or(&[]))
            })
        }
        (Some(&p), Some(&s)) => {
            seg_match(p, s) && match_segs(pat.get(1..).unwrap_or(&[]), path.get(1..).unwrap_or(&[]))
        }
        _ => false,
    }
}

fn seg_match(pat: &str, s: &str) -> bool {
    // `*` matches any run of chars within the segment.
    if !pat.contains('*') {
        return pat == s;
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let mut rest = s;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let Some(pos) = rest.find(part) else {
            return false;
        };
        if i == 0 && pos != 0 {
            return false;
        }
        rest = &rest[pos + part.len()..];
    }
    // Trailing `*` allows a suffix; otherwise the last part must end the string.
    if !pat.ends_with('*') && !rest.is_empty() {
        return false;
    }
    true
}

/// `grep { pattern, path?, cwd? }` — recursive regex-free substring search
/// under `cwd` (default workspace), returning `path:line:content` lines.
pub struct GrepTool {
    /// Workspace root for confinement.
    pub workspace: PathBuf,
}

impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("grep"),
            description: String::from("Search file contents for a substring"),
            schema: json!({"type":"object","required":["pattern"],"properties":{
                "pattern":{"type":"string"},"path":{"type":"string"},
                "cwd":{"type":"string"}}}),
        }
    }

    fn timeout(&self) -> Option<u64> {
        // Spec 12, R3: grep walks unbounded trees, so it is bounded.
        Some(30_000)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let pattern = require(args, "pattern")?;
        let cwd = resolve_cwd(&self.workspace, args.get("cwd").and_then(Value::as_str))?;
        let path_filter = args.get("path").and_then(Value::as_str);
        let mut out = String::new();
        let mut truncated = false;
        for file in walk_files(&cwd) {
            if let Some(pf) = path_filter
                && !glob_match(
                    pf,
                    &file
                        .strip_prefix(&cwd)
                        .unwrap_or(&file)
                        .display()
                        .to_string(),
                )
            {
                continue;
            }
            // Read through the capability gate so scope is enforced.
            let Ok(bytes) = ctx.fs()?.read(&file) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                if line.contains(pattern) {
                    let rel = file.strip_prefix(&cwd).unwrap_or(&file).display();
                    let entry = format!("{rel}:{}:{line}\n", i + 1);
                    if out.len() + entry.len() > MAX_OUTPUT {
                        truncated = true;
                        break;
                    }
                    out.push_str(&entry);
                }
            }
            if truncated {
                break;
            }
        }
        if truncated {
            out.push_str(TRUNCATION_MARKER);
        }
        Ok(json!({"matches": out, "truncated": truncated}))
    }
}

/// `glob { pattern, cwd? }` — list files under `cwd` (default workspace)
/// matching a glob pattern, one per line.
pub struct GlobTool {
    /// Workspace root for confinement.
    pub workspace: PathBuf,
}

impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("glob"),
            description: String::from("List files matching a glob pattern"),
            schema: json!({"type":"object","required":["pattern"],"properties":{
                "pattern":{"type":"string"},"cwd":{"type":"string"}}}),
        }
    }

    fn timeout(&self) -> Option<u64> {
        // Spec 12, R3: glob walks unbounded trees, so it is bounded.
        Some(30_000)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let pattern = require(args, "pattern")?;
        let cwd = resolve_cwd(&self.workspace, args.get("cwd").and_then(Value::as_str))?;
        // The pattern is relative to cwd; match against the relative path.
        let mut out = String::new();
        let mut truncated = false;
        for file in walk_files(&cwd) {
            let rel = file
                .strip_prefix(&cwd)
                .unwrap_or(&file)
                .display()
                .to_string();
            if glob_match(pattern, &rel) {
                let entry = format!("{rel}\n");
                if out.len() + entry.len() > MAX_OUTPUT {
                    truncated = true;
                    break;
                }
                out.push_str(&entry);
            }
        }
        if truncated {
            out.push_str(TRUNCATION_MARKER);
        }
        let _ = ctx; // glob is pure listing; no capability-gated read needed
        Ok(json!({"files": out, "truncated": truncated}))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::path::PathBuf;

    use pii_abi::PiiError;
    use pii_core::{Budget, CapKind, CapSet, Ctx, PathScope};

    use super::{GlobTool, GrepTool, glob_match};
    use crate::Tool;

    fn tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A root context granting `FS_READ` over exactly `allow`.
    fn read_ctx(allow: &[PathBuf]) -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::FS_READ;
        caps.fs_read = PathScope::new(allow.to_vec());
        Ctx::root(caps, Budget::unlimited())
    }

    #[test]
    fn glob_matcher_basic() {
        assert!(glob_match("*.rs", "a.rs"));
        assert!(!glob_match("*.rs", "a.txt"));
        assert!(glob_match("src/*.rs", "src/lib.rs"));
        assert!(glob_match("**/*.rs", "a/b/c.rs"));
        assert!(glob_match("**/*.rs", "lib.rs"));
        assert!(!glob_match("*.rs", "a/b.rs"));
    }

    #[test]
    fn grep_tool_finds_matches_in_workspace() {
        let dir = tempfile::Builder::new()
            .prefix("grep-tool-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        std::fs::write(canon.join("a.txt"), "hello world\nneedle here\n").unwrap();
        std::fs::write(canon.join("b.txt"), "no match\n").unwrap();
        let ctx = read_ctx(std::slice::from_ref(&canon));
        let tool = GrepTool { workspace: canon };
        let res = tool
            .call(&ctx, &serde_json::json!({"pattern": "needle"}))
            .unwrap();
        let matches = res["matches"].as_str().unwrap();
        assert!(matches.contains("a.txt:2:needle here"), "got {matches}");
        assert!(!matches.contains("b.txt"), "b.txt must not match");
    }

    #[test]
    fn grep_tool_denies_outside_workspace() {
        let dir = tempfile::Builder::new()
            .prefix("grep-deny-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let ctx = read_ctx(std::slice::from_ref(&canon));
        let tool = GrepTool { workspace: canon };
        let err = tool
            .call(&ctx, &serde_json::json!({"pattern": "x", "cwd": "/etc"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Denied(_)), "got {err:?}");
    }

    #[test]
    fn glob_tool_lists_matching_files() {
        let dir = tempfile::Builder::new()
            .prefix("glob-tool-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        std::fs::write(canon.join("a.rs"), "").unwrap();
        std::fs::write(canon.join("b.rs"), "").unwrap();
        std::fs::write(canon.join("c.txt"), "").unwrap();
        let ctx = read_ctx(std::slice::from_ref(&canon));
        let tool = GlobTool { workspace: canon };
        let res = tool
            .call(&ctx, &serde_json::json!({"pattern": "*.rs"}))
            .unwrap();
        let files = res["files"].as_str().unwrap();
        assert!(files.contains("a.rs"), "got {files}");
        assert!(files.contains("b.rs"), "got {files}");
        assert!(!files.contains("c.txt"), "c.txt must not match");
    }

    #[test]
    fn glob_tool_denies_outside_workspace() {
        let dir = tempfile::Builder::new()
            .prefix("glob-deny-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let ctx = read_ctx(std::slice::from_ref(&canon));
        let tool = GlobTool { workspace: canon };
        let err = tool
            .call(&ctx, &serde_json::json!({"pattern": "*", "cwd": "/etc"}))
            .unwrap_err();
        assert!(matches!(err, PiiError::Denied(_)), "got {err:?}");
    }
}
