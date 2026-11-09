//! `run` (spec 399 R1/R2): executes an argv exactly, never through `/bin/sh`.
//!
//! It uses the same `ctx.proc()` capability, workspace-bounded cwd, output cap and timeout as `bash`. Nothing is joined
//! into a string.

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_model::ToolSpec;
use serde_json::{Value, json};

use crate::Tool;
use crate::bash::{render, scoped_cwd, timeout_of};

/// Programs that read a command string or re-exec one. Matched on the final path component, the same list
/// `check_tool_surface` uses for the load check; the load check cannot see an argv that arrives as run input.
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh", "fish", "env"];

/// `run { argv: [string, ...], cwd?, timeout_ms? }` (spec 399 R1).
pub struct RunTool {
    /// Workspace the tool is bound to (cwd must stay inside).
    pub workspace: PathBuf,
}

impl Tool for RunTool {
    fn name(&self) -> &'static str {
        "run"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("run"),
            description: String::from(
                "Run a program with an argv array inside the workspace; no shell",
            ),
            schema: json!({"type":"object","required":["argv"],"properties":{
                "argv":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"},
                "timeout_ms":{"type":"integer"}}}),
        }
    }

    fn timeout(&self) -> Option<u64> {
        // Same bound as `bash`: a build or test run is the legitimate long case.
        Some(120_000)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let parts = argv_of(args)?;
        let program = parts.first().map_or("", String::as_str);
        let component = program.rsplit('/').next().unwrap_or(program);
        if SHELLS.contains(&component) {
            return Err(PiiError::Denied(format!(
                "program '{program}' is a shell; `run` executes a program, not a shell"
            )));
        }
        // The same list `bash` honours, applied to the program (see `deny_list_hit_argv`): one list, one message, one event.
        if let Some(entry) = crate::bash::deny_list_hit_argv(&parts) {
            return Err(PiiError::Denied(format!(
                "command matches deny list ('{entry}')"
            )));
        }
        let cwd = scoped_cwd(&self.workspace, args)?;
        let out = ctx.proc()?.run(&parts, &cwd, timeout_of(args))?;
        Ok(render(&out))
    }
}

/// The `argv` argument: a non-empty array of strings. A string is refused, never split (spec 399 Architecture).
fn argv_of(args: &Value) -> Result<Vec<String>, PiiError> {
    let items = args
        .get("argv")
        .and_then(Value::as_array)
        .ok_or_else(|| PiiError::Invalid(String::from("argv must be an array of strings")))?;
    let parsed = items
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| PiiError::Invalid(String::from("argv must be an array of strings")))
        })
        .collect::<Result<Vec<String>, PiiError>>()?;
    if parsed.is_empty() {
        return Err(PiiError::Invalid(String::from("argv is empty")));
    }
    Ok(parsed)
}
