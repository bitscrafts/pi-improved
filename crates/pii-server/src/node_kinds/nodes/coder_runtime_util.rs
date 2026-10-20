//! Transport-edge helpers for the coder runtime (spec 230): HTTP dechunking, podman error text,
//! image-probe verdicts, the env file path and the child wait. Split out of `coder_runtime.rs`
//! by the 450-line gate; the runtime's types stay there.

use std::sync::atomic::{AtomicU64, Ordering};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::Value;

/// The payload of a chunked body: each chunk's size line, its bytes, and a CRLF, up to the zero-size chunk.
pub(super) fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let eol = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("a chunk size line is unterminated")?;
        let size_line = String::from_utf8_lossy(rest.get(..eol).unwrap_or_default()).into_owned();
        let size = size_line
            .split(';')
            .next()
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok())
            .ok_or_else(|| format!("a chunk size is not hex: {size_line:?}"))?;
        rest = rest.get(eol + 2..).unwrap_or_default();
        if size == 0 {
            return Ok(out);
        }
        let chunk = rest.get(..size).ok_or("a chunk is truncated")?;
        out.extend_from_slice(chunk);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
}

/// The runtime's own message from an error body, or the body itself, collapsed to one line and cut to 240 chars.
pub(super) fn message(body: &str) -> String {
    let text = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| body.split_whitespace().collect::<Vec<_>>().join(" "));
    text.chars().take(240).collect()
}

/// Whether a failed probe's stderr says the image is absent, or says nothing (spec 228 R2, spec 231 R2). Podman's
/// absence words are `no such image` and `image not known`.
pub(super) fn probe_says_absent(stderr: &[u8]) -> bool {
    let said = String::from_utf8_lossy(stderr).to_lowercase();
    said.trim().is_empty() || said.contains("no such image") || said.contains("image not known")
}

/// Podman's own stderr from a failed probe, collapsed to one line and cut to its last 240 characters (spec 228 R2).
/// The absence clause stands alone only when podman said nothing at all.
pub(super) fn probe_reason(stderr: &[u8]) -> String {
    let collapsed = String::from_utf8_lossy(stderr)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let tail: String = collapsed.chars().rev().take(240).collect::<String>();
    let tail = tail.chars().rev().collect::<String>();
    if tail.is_empty() {
        "image inspect failed with no stderr".to_owned()
    } else {
        tail
    }
}

/// A fresh env-file path under the runs dir, unique within this process (spec 227 R6).
///
/// The file is created later with `create_new`, so an existing path is an error, never overwritten. No runs dir is a
/// refusal: the secrets have nowhere private to go.
///
/// # Errors
/// The reason string when no runs dir is configured or it cannot be created.
pub(crate) fn env_file_path(runs_dir: Option<&Path>) -> Result<PathBuf, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let Some(runs) = runs_dir else {
        return Err(String::from("env file: no runs directory is configured"));
    };
    std::fs::create_dir_all(runs).map_err(|e| format!("env file: {e}"))?;
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    Ok(runs.join(format!("pii-coder-{}-{seq}.env", std::process::id())))
}

/// The names of an environment, never its values.
pub(super) fn env_names(env: &[(String, String)]) -> Vec<String> {
    env.iter().map(|(name, _)| name.clone()).collect()
}

/// Milliseconds since `started`, saturating: a duration never wraps negative.
#[must_use]
pub(crate) fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
