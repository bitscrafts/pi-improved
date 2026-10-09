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

/// The deny-list entry `command` matches, if any.
#[must_use]
pub fn deny_list_hit(command: &str) -> Option<&'static str> {
    DENY_LIST.iter().find(|d| command.contains(**d)).copied()
}

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
        if let Some(d) = deny_list_hit(command) {
            return Err(PiiError::Denied(format!(
                "command matches deny list ('{d}')"
            )));
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
    // R1: the cut is byte-exact; when a multibyte char straddles MAX_OUTPUT,
    // the straddling bytes (up to 3) are dropped by the lossy decode.
    let cap = |b: &[u8]| -> String {
        let capped: Option<&[u8]> = if b.len() > MAX_OUTPUT {
            b.get(..MAX_OUTPUT)
        } else {
            Some(b)
        };
        let capped = capped.unwrap_or(b);
        let mut s = String::from_utf8_lossy(capped).to_string();
        if b.len() > MAX_OUTPUT {
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::path::PathBuf;

    use pii_core::{ArgvScope, Budget, CapKind, CapSet, ProcOutput};
    use serde_json::json;

    use super::{BashTool, MAX_OUTPUT, TRUNCATION_MARKER, render};
    use crate::Tool;

    fn tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A root context granting `PROC` for `/bin/sh` only.
    fn proc_ctx() -> pii_core::Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::PROC;
        caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
        pii_core::Ctx::root(caps, Budget::unlimited())
    }

    #[test]
    fn render_truncates_large_output() {
        let out = ProcOutput {
            exit_code: 0,
            stdout: vec![b'a'; MAX_OUTPUT + 10],
            stderr: Vec::new(),
            timed_out: false,
            cancelled: false,
        };
        let v = render(&out);
        assert_eq!(v["truncated"], json!(true));
        let text = v["output"].as_str().unwrap();
        // Truncated at the cap, marker appended after it (marker is ASCII so
        // byte-truncation is safe).
        assert_eq!(text.len(), MAX_OUTPUT + TRUNCATION_MARKER.len());
        assert!(text.ends_with(TRUNCATION_MARKER));
        // The stdout field is capped independently (cap + marker).
        let so = v["stdout"].as_str().unwrap();
        assert_eq!(so.len(), MAX_OUTPUT + TRUNCATION_MARKER.len());
        assert!(so.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn render_untruncated_when_small() {
        let out = ProcOutput {
            exit_code: 3,
            stdout: b"hello".to_vec(),
            stderr: b" world".to_vec(),
            timed_out: false,
            cancelled: false,
        };
        let v = render(&out);
        assert_eq!(v["truncated"], json!(false));
        assert_eq!(v["output"].as_str().unwrap(), "hello world");
        assert_eq!(v["exit_code"], json!(3));
        assert_eq!(v["stdout"].as_str().unwrap(), "hello");
        assert_eq!(v["stderr"].as_str().unwrap(), " world");
        assert!(!v["output"].as_str().unwrap().contains("[...truncated...]"));
    }

    #[test]
    fn renders_multibyte_output_at_the_cap() {
        // 65535 ASCII bytes + four `ç` (2 bytes each, len 65539): the old
        // decoded-String truncate panicked on the 65536 byte cut.
        let mut stdout = vec![b'a'; MAX_OUTPUT - 1];
        stdout.extend_from_slice("çççç".as_bytes());
        let out = ProcOutput {
            exit_code: 0,
            stdout,
            stderr: Vec::new(),
            timed_out: false,
            cancelled: false,
        };
        // Must not panic. Byte-truncation of the combined output happens
        // BEFORE the lossy decode (unchanged pre-existing path), so a
        // straddling partial `ç` byte becomes U+FFFD (3 bytes, +2 over the
        // original 2-byte char): decoded length ≤ MAX_OUTPUT + 2, plus the
        // marker. NOTE: the spec's acceptance row said "≤ MAX_OUTPUT +
        // marker"; that bound is off by the FFFD expansion (+2 here).
        let v = render(&out);
        let text = v["output"].as_str().unwrap();
        assert!(text.len() <= MAX_OUTPUT + TRUNCATION_MARKER.len() + 2);
        assert_eq!(v["truncated"], json!(true));
        // The stdout field cap is byte slice truncation now too: it must not
        // panic and stays ≤ cap (a straddling multibyte char may lose its
        // straddling bytes — accepted, keeps the cap byte-exact; the lossy
        // decode can replace those bytes with U+FFFD, net length change ≤ +2,
        // and the marker is appended after).
        let so = v["stdout"].as_str().unwrap();
        assert!(so.len() <= MAX_OUTPUT + TRUNCATION_MARKER.len() + 2);
        assert!(so.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn render_boundary_ascii_unchanged() {
        let out = ProcOutput {
            exit_code: 0,
            stdout: vec![b'a'; MAX_OUTPUT + MAX_OUTPUT / 2],
            stderr: Vec::new(),
            timed_out: false,
            cancelled: false,
        };
        let v = render(&out);
        let so = v["stdout"].as_str().unwrap();
        // Cut at exactly MAX_OUTPUT, marker appended.
        assert_eq!(so.len(), MAX_OUTPUT + TRUNCATION_MARKER.len());
        assert!(so.ends_with(TRUNCATION_MARKER));
        assert_eq!(so.as_bytes()[MAX_OUTPUT - 1], b'a');
    }

    #[test]
    fn render_caps_both_streams_independently() {
        let out = ProcOutput {
            exit_code: 0,
            stdout: vec![b's'; MAX_OUTPUT + 5],
            stderr: vec![b'e'; MAX_OUTPUT + 5],
            timed_out: false,
            cancelled: false,
        };
        let v = render(&out);
        let so = v["stdout"].as_str().unwrap();
        let se = v["stderr"].as_str().unwrap();
        assert_eq!(so.len(), MAX_OUTPUT + TRUNCATION_MARKER.len());
        assert_eq!(se.len(), MAX_OUTPUT + TRUNCATION_MARKER.len());
        assert_eq!(v["truncated"], json!(true));
    }

    #[test]
    fn bash_denies_dangerous_command() {
        let dir = tempfile::Builder::new()
            .prefix("bash-deny-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let ctx = proc_ctx();
        let tool = BashTool { workspace: canon };
        for cmd in ["sudo rm -rf /", "echo ok && reboot"] {
            let err = tool.call(&ctx, &json!({"command": cmd})).unwrap_err();
            assert!(
                matches!(err, pii_abi::PiiError::Denied(_)),
                "'{cmd}' must be denied, got {err:?}"
            );
        }
    }

    #[test]
    fn bash_denies_cwd_outside_workspace() {
        let dir = tempfile::Builder::new()
            .prefix("bash-cwd-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let ctx = proc_ctx();
        let tool = BashTool { workspace: canon };
        let err = tool
            .call(&ctx, &json!({"command": "echo hi", "cwd": "/etc"}))
            .unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Denied(_)), "got {err:?}");
    }

    #[test]
    fn bash_runs_real_command() {
        let dir = tempfile::Builder::new()
            .prefix("bash-run-")
            .tempdir_in(tmp_root())
            .unwrap();
        let canon = dunce::canonicalize(dir.path()).unwrap();
        let ctx = proc_ctx();
        let tool = BashTool { workspace: canon };
        let res = tool
            .call(&ctx, &json!({"command": "echo hello"}))
            .expect("echo must run");
        assert_eq!(res["exit_code"], json!(0));
        assert!(res["output"].as_str().unwrap().contains("hello"));
        assert_eq!(res["truncated"], json!(false));
    }
}
