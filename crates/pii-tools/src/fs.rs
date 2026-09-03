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
