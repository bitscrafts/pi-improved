//! Filesystem tools (F3.6). All file access goes through `Ctx::fs()` —
//! never `std::fs` directly — so the capability gate is enforced in one place.

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_model::ToolSpec;
use serde_json::{Value, json};

use crate::Tool;

fn require<'a>(args: &'a Value, key: &str) -> Result<&'a str, PiiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| PiiError::Invalid(format!("{key} must be a string")))
}

/// Resolves a tool path against the workspace: relative paths are joined to
/// the workspace root so the capability scope check sees the canonical
/// location the file would occupy.
fn resolve_path(workspace: &std::path::Path, p: &std::path::Path) -> std::path::PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(p)
    }
}

/// Reads a text file (F3.6 `fs_read { path, offset?, limit? }`).
pub struct FsReadTool {
    /// Workspace root for relative-path resolution.
    pub workspace: std::path::PathBuf,
}

impl Tool for FsReadTool {
    fn name(&self) -> &'static str {
        "fs_read"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("fs_read"),
            description: String::from("Read a text file"),
            schema: json!({"type":"object","required":["path"],"properties":{
                "path":{"type":"string"},"offset":{"type":"integer"},
                "limit":{"type":"integer"}}}),
        }
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let path = resolve_path(
            &self.workspace,
            std::path::Path::new(require(args, "path")?),
        );
        let bytes = ctx.fs()?.read(&path)?;
        let text = String::from_utf8_lossy(&bytes);
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let limit = args.get("limit").and_then(Value::as_u64);
        let selected: Vec<&str> = text
            .lines()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            .take(
                limit
                    .and_then(|l| usize::try_from(l).ok())
                    .unwrap_or(usize::MAX),
            )
            .collect();
        Ok(json!({"content": selected.join("\n"), "total_lines": text.lines().count()}))
    }
}

/// Writes a text file (F3.6 `fs_write { path, content }`).
pub struct FsWriteTool {
    /// Workspace root for relative-path resolution.
    pub workspace: std::path::PathBuf,
}

impl Tool for FsWriteTool {
    fn name(&self) -> &'static str {
        "fs_write"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("fs_write"),
            description: String::from("Write a text file"),
            schema: json!({"type":"object","required":["path","content"],"properties":{
                "path":{"type":"string"},"content":{"type":"string"}}}),
        }
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let path = resolve_path(
            &self.workspace,
            std::path::Path::new(require(args, "path")?),
        );
        let content = require(args, "content")?;
        ctx.fs()?.write(&path, content.as_bytes())?;
        Ok(json!({"written": content.len()}))
    }
}

/// Replaces `old_string` with `new_string`; fails unless the match is unique.
pub struct EditTool {
    /// Workspace root for relative-path resolution.
    pub workspace: std::path::PathBuf,
}

impl Tool for EditTool {
    fn name(&self) -> &'static str {
        "edit"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("edit"),
            description: String::from("Replace one unique string in a file"),
            schema: json!({"type":"object","required":["path","old_string","new_string"],
                "properties":{"path":{"type":"string"},"old_string":{"type":"string"},
                "new_string":{"type":"string"}}}),
        }
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let path = resolve_path(
            &self.workspace,
            std::path::Path::new(require(args, "path")?),
        );
        let old = require(args, "old_string")?;
        let new = require(args, "new_string")?;
        let bytes = ctx.fs()?.read(&path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| PiiError::Invalid(String::from("file is not UTF-8")))?;
        if text.matches(old).count() != 1 {
            return Err(PiiError::Invalid(format!(
                "old_string occurs {} times (need exactly 1)",
                text.matches(old).count()
            )));
        }
        let updated = text.replacen(old, new, 1);
        ctx.fs()?.write(&path, updated.as_bytes())?;
        Ok(json!({"edited": true}))
    }
}

/// Applies ordered edits all-or-nothing, writing the file exactly once.
pub struct MultiEditTool {
    /// Workspace root for relative-path resolution.
    pub workspace: std::path::PathBuf,
}

impl Tool for MultiEditTool {
    fn name(&self) -> &'static str {
        "multi_edit"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("multi_edit"),
            description: String::from("Apply ordered edits atomically"),
            schema: json!({"type":"object","required":["path","edits"],"properties":{
                "path":{"type":"string"},"edits":{"type":"array"}}}),
        }
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let path = resolve_path(
            &self.workspace,
            std::path::Path::new(require(args, "path")?),
        );
        let edits = args
            .get("edits")
            .and_then(Value::as_array)
            .ok_or_else(|| PiiError::Invalid(String::from("edits must be an array")))?;
        let bytes = ctx.fs()?.read(&path)?;
        let mut text = String::from_utf8(bytes)
            .map_err(|_| PiiError::Invalid(String::from("file is not UTF-8")))?;
        for e in edits {
            let old = e
                .get("old_string")
                .and_then(Value::as_str)
                .ok_or_else(|| PiiError::Invalid(String::from("edit needs old_string")))?;
            let new = e
                .get("new_string")
                .and_then(Value::as_str)
                .ok_or_else(|| PiiError::Invalid(String::from("edit needs new_string")))?;
            if text.matches(old).count() != 1 {
                return Err(PiiError::Invalid(format!(
                    "old_string occurs {} times (need exactly 1)",
                    text.matches(old).count()
                )));
            }
            text = text.replacen(old, new, 1);
        }
        ctx.fs()?.write(&path, text.as_bytes())?;
        Ok(json!({"edits_applied": edits.len()}))
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
    use serde_json::json;

    use super::{EditTool, FsReadTool, FsWriteTool, MultiEditTool};
    use crate::Tool;

    fn tmp_root() -> PathBuf {
        PathBuf::from("/projects/pi-improved/_tmp")
    }

    /// A root context granting `FS_READ | FS_WRITE` over exactly `allow`.
    fn fs_ctx(allow: &[PathBuf]) -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::FS_READ | CapKind::FS_WRITE;
        caps.fs_read = PathScope::new(allow.to_vec());
        caps.fs_write = PathScope::new(allow.to_vec());
        Ctx::root(caps, Budget::unlimited())
    }

    fn tempdir(prefix: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        (dir, canon)
    }

    #[test]
    fn fs_read_respects_offset_limit() {
        let (_dir, canon) = tempdir("fs-read-");
        std::fs::write(canon.join("lines.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let ctx = fs_ctx(std::slice::from_ref(&canon));
        let tool = FsReadTool { workspace: canon };
        let res = tool
            .call(&ctx, &json!({"path": "lines.txt", "offset": 1, "limit": 2}))
            .unwrap();
        assert_eq!(res["content"].as_str().unwrap(), "l2\nl3");
        assert_eq!(res["total_lines"], json!(5));
        // No offset/limit: full content.
        let all = tool.call(&ctx, &json!({"path": "lines.txt"})).unwrap();
        assert_eq!(all["content"].as_str().unwrap(), "l1\nl2\nl3\nl4\nl5");
        // Offset past the end: empty content, total unchanged.
        let past = tool
            .call(&ctx, &json!({"path": "lines.txt", "offset": 99}))
            .unwrap();
        assert_eq!(past["content"].as_str().unwrap(), "");
        assert_eq!(past["total_lines"], json!(5));
    }

    #[test]
    fn fs_write_then_read_roundtrip() {
        let (_dir, canon) = tempdir("fs-write-");
        let ctx = fs_ctx(std::slice::from_ref(&canon));
        let w = FsWriteTool {
            workspace: canon.clone(),
        };
        let r = FsReadTool { workspace: canon };
        let res = w
            .call(&ctx, &json!({"path": "out.txt", "content": "abc"}))
            .unwrap();
        assert_eq!(res["written"], json!(3));
        let got = r.call(&ctx, &json!({"path": "out.txt"})).unwrap();
        assert_eq!(got["content"].as_str().unwrap(), "abc");
    }

    #[test]
    fn edit_requires_exactly_one_match() {
        let (_dir, canon) = tempdir("edit-miss-");
        std::fs::write(canon.join("f.txt"), "alpha beta alpha\n").unwrap();
        let ctx = fs_ctx(std::slice::from_ref(&canon));
        let tool = EditTool { workspace: canon };
        // 0 matches (no "gamma").
        let err = tool
            .call(
                &ctx,
                &json!({"path": "f.txt", "old_string": "gamma", "new_string": "x"}),
            )
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        // 2 matches ("alpha" twice).
        let err = tool
            .call(
                &ctx,
                &json!({"path": "f.txt", "old_string": "alpha", "new_string": "x"}),
            )
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
    }

    #[test]
    fn edit_succeeds_on_unique_match() {
        let (_dir, canon) = tempdir("edit-hit-");
        std::fs::write(canon.join("f.txt"), "alpha beta\n").unwrap();
        let ctx = fs_ctx(std::slice::from_ref(&canon));
        let tool = EditTool {
            workspace: canon.clone(),
        };
        let res = tool
            .call(
                &ctx,
                &json!({"path": "f.txt", "old_string": "beta", "new_string": "gamma"}),
            )
            .unwrap();
        assert_eq!(res["edited"], json!(true));
        let on_disk = std::fs::read_to_string(canon.join("f.txt")).unwrap();
        assert_eq!(on_disk, "alpha gamma\n");
    }

    #[test]
    fn multi_edit_is_all_or_nothing() {
        let (_dir, canon) = tempdir("multi-edit-");
        std::fs::write(canon.join("f.txt"), "one two\n").unwrap();
        let ctx = fs_ctx(std::slice::from_ref(&canon));
        let tool = MultiEditTool {
            workspace: canon.clone(),
        };
        // First edit would succeed, second fails (no match) → file unchanged.
        let err = tool
            .call(
                &ctx,
                &json!({"path": "f.txt", "edits": [
                    {"old_string": "one", "new_string": "ONE"},
                    {"old_string": "nope", "new_string": "X"}
                ]}),
            )
            .unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)), "got {err:?}");
        let on_disk = std::fs::read_to_string(canon.join("f.txt")).unwrap();
        assert_eq!(on_disk, "one two\n", "file must be untouched on failure");
        // All-or-nothing means even the successful prefix was never written.
        // A valid run then applies both edits.
        let res = tool
            .call(
                &ctx,
                &json!({"path": "f.txt", "edits": [
                    {"old_string": "one", "new_string": "ONE"},
                    {"old_string": "two", "new_string": "TWO"}
                ]}),
            )
            .unwrap();
        assert_eq!(res["edits_applied"], json!(2));
        assert_eq!(
            std::fs::read_to_string(canon.join("f.txt")).unwrap(),
            "ONE TWO\n"
        );
    }
}
