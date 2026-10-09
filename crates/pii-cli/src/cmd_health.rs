//! `pii health [--json]` (spec 99 R6): prints the `sys.status` identity
//! payload from the running daemon. Human-readable by default; `--json`
//! prints the payload verbatim. Exits non-zero with a clear message when no
//! daemon answers. Output goes through `writeln!` on the std handles, not the
//! raw print macros the observability lint bans (spec 98).

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::ExitCode;

use pii_proto::method::Method;

use crate::cmd_ctl::{next_str, run_call};

/// Text at JSON pointer `ptr` in `v`, or `-` when absent.
fn txt(v: &serde_json::Value, ptr: &str) -> String {
    match v.pointer(ptr) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => "-".to_owned(),
    }
}

/// Renders the identity-bearing parts of a `sys.status` payload as text.
fn render(v: &serde_json::Value) -> String {
    let mut out = format!(
        "pii {} ({}, {})\nconfig: {} sha256={}\nstores:\n",
        txt(v, "/build/version"),
        txt(v, "/build/revision"),
        txt(v, "/build/target"),
        txt(v, "/config/path"),
        txt(v, "/config/sha256"),
    );
    for st in v
        .pointer("/stores")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let _ = writeln!(
            out,
            "  {}: {} ({} bytes, format_version={})",
            txt(st, "/name"),
            txt(st, "/path"),
            txt(st, "/bytes"),
            txt(st, "/format_version"),
        );
    }
    out.push_str("mounts:\n");
    for m in v
        .pointer("/mounts")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let _ = writeln!(
            out,
            "  {} [{}]: {}  ({})",
            txt(m, "/name"),
            txt(m, "/mode"),
            txt(m, "/root"),
            txt(m, "/policy"),
        );
    }
    out.push_str("counts:\n");
    for (k, _) in v
        .pointer("/counts")
        .and_then(|c| c.as_object())
        .into_iter()
        .flatten()
    {
        let _ = writeln!(out, "  {k}: {}", txt(v, &format!("/counts/{k}")));
    }
    out
}

/// `pii health [--json]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_health(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut json = false;
    while let Some(a) = next_str(args) {
        if a == "--json" {
            json = true;
        } else {
            let _ = writeln!(std::io::stderr(), "usage: pii health [--json]");
            return ExitCode::from(2);
        }
    }
    match run_call(Method::SysStatus, serde_json::json!({})) {
        Ok(v) => {
            if json {
                let _ = writeln!(std::io::stdout(), "{v}");
            } else {
                let _ = write!(std::io::stdout(), "{}", render(&v));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(
                std::io::stderr(),
                "error: no daemon answered ({e}); is `pii daemon` running?"
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::render;

    #[test]
    fn render_shows_store_path_mount_root_and_config_hash() {
        let v = serde_json::json!({
            "build": {"version": "0.1.0", "revision": "abc", "target": "t"},
            "config": {"path": "/c/pii.toml", "sha256": "ff00"},
            "stores": [{"name": "main", "path": "/s/store.redb", "bytes": 8192, "format_version": 1}],
            "mounts": [{"name": "workspace", "root": "/w", "mode": "rw", "policy": "p"}],
            "counts": {"runs": 3}
        });
        let text = render(&v);
        for needle in [
            "/s/store.redb",
            "format_version=1",
            "/w",
            "ff00",
            "runs: 3",
            "0.1.0",
        ] {
            assert!(text.contains(needle), "missing {needle} in:\n{text}");
        }
    }
}
