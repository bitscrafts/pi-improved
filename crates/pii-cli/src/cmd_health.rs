//! `pii health [--json]` (spec 99 R6): prints the `sys.status` identity
//! payload from the running daemon. Human-readable by default; `--json`
//! prints the payload verbatim. Output goes through `writeln!` on the std
//! handles, not the raw print macros the observability lint bans (spec 98).
//!
//! Exit codes (spec 325 R1): `0` the daemon answers from this client's build
//! revision, `2` no daemon answers, `3` a daemon that is not this build, or a
//! comparison that cannot be made because a revision is unknown.

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::ExitCode;

use pii_proto::method::Method;
use pii_server::daemon::identity::REVISION;

use crate::cmd_ctl::{next_str, run_call};

/// What the daemon's revision says about this client's build (spec 325 R1, R6).
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Same known revision on both sides.
    Same,
    /// Two different known revisions: the running binary is not this tree.
    Drift,
    /// One side is missing, empty, or `unknown`; names that side.
    Unverified(&'static str),
}

/// A revision that can prove anything: not missing, not empty, not `unknown`.
fn known(rev: &str) -> bool {
    !rev.is_empty() && rev != "unknown"
}

/// Compares the daemon's revision with the client's. Unknown is never agreement.
fn verdict(daemon: &str, client: &str) -> Verdict {
    if !known(daemon) {
        Verdict::Unverified("daemon")
    } else if !known(client) {
        Verdict::Unverified("client")
    } else if daemon == client {
        Verdict::Same
    } else {
        Verdict::Drift
    }
}

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
        "pii {} ({}, {})\nclient: {REVISION}\nconfig: {} sha256={}\nstores:\n",
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
    if v.pointer("/since_deploy").is_some() {
        out.push_str("since_deploy:\n");
        for k in [
            "revision",
            "runs",
            "armed",
            "unattributed",
            "posted",
            "post_refused",
            "unreadable",
            "starts",
            "hard_deaths",
        ] {
            let _ = writeln!(out, "  {k}: {}", txt(v, &format!("/since_deploy/{k}")));
        }
        for k in ["running", "suspended", "succeeded", "failed"] {
            let _ = writeln!(
                out,
                "  runs_by_status.{k}: {}",
                txt(v, &format!("/since_deploy/runs_by_status/{k}"))
            );
        }
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
            let daemon = v
                .pointer("/build/revision")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            match verdict(daemon, REVISION) {
                Verdict::Same => ExitCode::SUCCESS,
                Verdict::Drift => {
                    let _ = writeln!(
                        std::io::stderr(),
                        "DRIFT: the daemon answers from {daemon}; this client is {REVISION} — the running binary is not this tree"
                    );
                    ExitCode::from(3)
                }
                Verdict::Unverified(side) => {
                    let _ = writeln!(
                        std::io::stderr(),
                        "UNVERIFIED: {side} build revision is unknown — this client cannot prove the daemon is this build"
                    );
                    ExitCode::from(3)
                }
            }
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

    use super::{Verdict, render, verdict};

    #[test]
    fn health_verdict_same() {
        assert_eq!(verdict("20f967e1a9a9", "20f967e1a9a9"), Verdict::Same);
    }

    #[test]
    fn health_verdict_drift() {
        assert_eq!(verdict("20f967e1a9a9", "de83a8c6b69d"), Verdict::Drift);
    }

    #[test]
    fn health_verdict_unknown_daemon() {
        assert_eq!(
            verdict("unknown", "de83a8c6b69d"),
            Verdict::Unverified("daemon")
        );
    }

    #[test]
    fn health_verdict_unknown_client() {
        assert_eq!(
            verdict("de83a8c6b69d", "unknown"),
            Verdict::Unverified("client")
        );
    }

    #[test]
    fn health_verdict_missing() {
        assert_eq!(verdict("", "de83a8c6b69d"), Verdict::Unverified("daemon"));
        assert_eq!(verdict("de83a8c6b69d", ""), Verdict::Unverified("client"));
    }

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

    #[test]
    fn since_deploy_prints_starts_and_hard_deaths() {
        let v = serde_json::json!({
            "since_deploy": {"revision": "abc", "starts": 3, "hard_deaths": 1}
        });
        let text = render(&v);
        for needle in ["starts: 3", "hard_deaths: 1"] {
            assert!(text.contains(needle), "missing {needle} in:\n{text}");
        }
    }
}
