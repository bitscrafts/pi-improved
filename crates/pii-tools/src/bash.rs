//! Bash mediation (F3.7): deny list → cwd scope check → `ProcCap::run`
//! under the (possibly sandboxed) context → output capping.

use std::path::Path;
use std::time::Duration;

use pii_abi::PiiError;
use pii_core::{Ctx, ProcOutput, env_var};
use pii_model::ToolSpec;
use serde_json::{Value, json};

use crate::Tool;

const DENY_LIST: &[&str] = &[
    "rm -rf /",
    ":(){",
    "mkfs",
    "dd of=/dev",
    "shutdown",
    "reboot",
    "sudo",
];
const MAX_OUTPUT: usize = 64 * 1024;
const TRUNCATION_MARKER: &str = "\n[...truncated...]";

/// `bash { command, cwd?, timeout_ms? }` (F3.7).
pub struct BashTool {
    /// Workspace the tool is bound to (cwd must stay inside).
    pub workspace: std::path::PathBuf,
}

impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: String::from("bash"),
            description: String::from("Run a shell command inside the workspace"),
            schema: json!({"type":"object","required":["command"],"properties":{
                "command":{"type":"string"},"cwd":{"type":"string"},
                "timeout_ms":{"type":"integer"}}}),
        }
    }

    fn timeout(&self) -> Option<u64> {
        // Spec 12, R3: bash is the one tool that can legitimately run long
        // (builds, test suites), so it gets the loosest declared bound.
        Some(120_000)
    }

    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError> {
        let command = require_str(args, "command")?;
        for d in DENY_LIST {
            if command.contains(d) {
                return Err(PiiError::Denied(format!(
                    "command matches deny list ('{d}')"
                )));
            }
        }
        let cwd = match args.get("cwd").and_then(Value::as_str) {
            Some(c) => {
                let canon = dunce::canonicalize(c)
                    .map_err(|e| PiiError::Invalid(format!("cwd {c}: {e}")))?;
                if !canon.starts_with(&self.workspace) {
                    return Err(PiiError::Denied(format!(
                        "cwd {c} is outside the workspace"
                    )));
                }
                canon
            }
            None => self.workspace.clone(),
        };
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map_or(120_000u64, |t| t.min(600_000));
        let shell_argv = vec![
            String::from("/bin/sh"),
            String::from("-c"),
            command.to_owned(),
        ];
        let out = ctx
            .proc()?
            .run(&shell_argv, &cwd, Duration::from_millis(timeout_ms))?;
        Ok(render(&out))
    }
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, PiiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| PiiError::Invalid(format!("{key} must be a string")))
}

/// Caps combined output at 64 KiB with a marker (F3.7 step 5). The
/// `stdout`/`stderr` fields are also capped so the cap cannot be bypassed
/// through them (F16, opus review).
#[must_use]
pub fn render(out: &ProcOutput) -> Value {
    let mut combined = out.stdout.clone();
    combined.extend_from_slice(&out.stderr);
    let truncated = combined.len() > MAX_OUTPUT;
    if truncated {
        combined.truncate(MAX_OUTPUT);
    }
    let mut text = String::from_utf8_lossy(&combined).to_string();
    if truncated {
        text.push_str(TRUNCATION_MARKER);
    }
    let cap = |b: &[u8]| -> String {
        let mut s = String::from_utf8_lossy(b).to_string();
        if s.len() > MAX_OUTPUT {
            s.truncate(MAX_OUTPUT);
            s.push_str(TRUNCATION_MARKER);
        }
        s
    };
    json!({
        "exit_code": out.exit_code,
        "stdout": cap(&out.stdout),
        "stderr": cap(&out.stderr),
        "truncated": truncated,
        "output": text,
    })
}

/// Default timeout from `$PII_BASH_TIMEOUT_MS` or 120 s (helper for callers).
#[must_use]
pub fn default_timeout() -> Duration {
    Duration::from_millis(
        env_var("PII_BASH_TIMEOUT_MS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(120_000),
    )
}

/// True when `p` is inside `workspace` after canonicalisation (test helper
/// shared with the sandbox module).
#[must_use]
pub fn inside_workspace(workspace: &Path, p: &Path) -> bool {
    p.starts_with(workspace)
}
