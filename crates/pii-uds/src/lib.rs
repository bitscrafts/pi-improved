//! Client-side UDS transport helpers (B2, 011 A.11).
//!
//! The client/server split (002 §12.2) is machine-checkable: a client crate
//! (`pii-tui`, `pii-web`) must not pull the daemon tree (`pii-graph`,
//! `pii-store`, redb) into its dependency graph. `pii-web` previously took
//! `pii-server` as a direct dependency for exactly two helpers —
//! `client_connect` and `runtime_dir` — which dragged the whole daemon tree
//! into a read-only viewer. This crate hosts those two helpers so clients
//! depend on `pii-uds` (lean) instead of `pii-server` (heavy).
//!
//! The daemon-side socket machinery (`prepare_socket`, `authorize_peer`, the
//! testkit bind/connect) stays in `pii-server::uds`; only the client-facing
//! resolution and connect live here.

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_observe::{Event, Level, NullObserver, Observer, names};

/// Resolves the runtime dir (F2.8, extended 2026-09-05; spec 339).
///
/// Precedence: `$PII_RUNTIME_DIR` → `[core].runtime_dir` in the persistent
/// config file (see `pii-server`'s `Config::default_path`) →
/// `/run/user/<uid>/pii`. The `instance` argument no longer names a path
/// segment: the daemon and every client resolve the same directory, which is
/// the one the shipped user unit pins.
/// The config-file lookup is a plain best-effort read (a malformed config
/// here does not abort — `pii daemon`'s own startup path already surfaces
/// config parse errors loudly; a lookup helper silently falling back to the
/// next candidate is the right behavior for callers like `pii sys status`
/// that just need SOME runtime dir to try).
#[must_use]
pub fn runtime_dir(_instance: &str) -> Option<PathBuf> {
    Some(resolve_runtime_dir(
        pii_core::env_var("PII_RUNTIME_DIR"),
        config_runtime_dir(&default_config_path()),
        current_uid(),
    ))
}

/// The one resolution rule: an explicit directory wins verbatim, then the
/// configured one, then the uid-derived default.
#[must_use]
pub fn resolve_runtime_dir(
    explicit: Option<String>,
    configured: Option<PathBuf>,
    uid: u32,
) -> PathBuf {
    explicit
        .map(PathBuf::from)
        .or(configured)
        .unwrap_or_else(|| uid_runtime_dir(uid))
}

/// The derived default for `uid`: `/run/user/<uid>/pii`.
#[must_use]
pub fn uid_runtime_dir(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}/pii"))
}

/// The default config file path (`$PII_CONFIG` else `$HOME/.pii/pii.toml`),
/// mirroring `pii-server`'s `Config::default_path` without pulling the
/// daemon config type into a client crate.
fn default_config_path() -> PathBuf {
    if let Some(p) = pii_core::env_var("PII_CONFIG") {
        return PathBuf::from(p);
    }
    let home = pii_core::env_var("HOME").unwrap_or_else(|| String::from("/root"));
    PathBuf::from(home).join(".pii").join("pii.toml")
}

/// Best-effort `[core].runtime_dir` from the given config file. A malformed
/// or absent file yields `None` (the caller falls through to the next
/// candidate).
fn config_runtime_dir(path: &std::path::Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let v: toml::Value = toml::from_str(&raw).ok()?;
    v.get("core")
        .and_then(|c| c.get("runtime_dir"))
        .and_then(toml::Value::as_str)
        .map(PathBuf::from)
}

/// The effective uid of this process (from `/proc/self/status`).
#[must_use]
pub fn current_uid() -> u32 {
    // rustix is intentionally not a dependency of this build; `nobody`-style
    // uid discovery goes through the libc-free path: `id -u` is overkill, so
    // fall back to 0 when `/proc/self/status` cannot be read.
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                let first = rest.split_whitespace().next().unwrap_or("0");
                return first.parse().unwrap_or(0);
            }
        }
    }
    0
}

/// Client-side UDS connect.
///
/// The ONE `UnixStream::connect` site outside the daemon bind (spec F2.13
/// allowlist keeps this file as a sanctioned transport file). Reports to nobody; see [`client_connect_observed`].
///
/// # Errors
/// [`PiiError::Internal`] on connect failure.
pub async fn client_connect(sock: &std::path::Path) -> Result<tokio::net::UnixStream, PiiError> {
    client_connect_observed(sock, &NullObserver).await
}

/// [`client_connect`], emitting [`names::UDS_CONNECT_FAILED`] (`endpoint`,
/// `error`) to `observer` when the socket cannot be reached (spec 109 R7).
///
/// # Errors
/// [`PiiError::Internal`] on connect failure.
pub async fn client_connect_observed(
    sock: &std::path::Path,
    observer: &dyn Observer,
) -> Result<tokio::net::UnixStream, PiiError> {
    tokio::net::UnixStream::connect(sock).await.map_err(|e| {
        observer.event(
            &Event::new("uds", Level::Error, names::UDS_CONNECT_FAILED)
                .field("endpoint", sock.display().to_string())
                .field("error", e.to_string()),
        );
        PiiError::Internal(format!("connect {}: {e}", sock.display()))
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    /// Test scratch root inside the tree (`<crate>/_tmp`), never `/tmp`.
    fn scratch_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("_tmp")
    }

    #[tokio::test]
    async fn connect_failure_is_emitted_with_the_endpoint() {
        let rec = pii_observe::Recorder::new();
        let sock = scratch_root().join("pii-uds-no-such-socket-109");
        let err = super::client_connect_observed(&sock, &rec).await.err();
        assert!(err.is_some(), "connecting to a missing socket must fail");
        let ev = rec.events_named(pii_observe::names::UDS_CONNECT_FAILED);
        assert_eq!(ev.len(), 1);
        assert_eq!(
            ev[0].fields.get("endpoint").and_then(|v| v.as_str()),
            Some(sock.display().to_string().as_str())
        );
    }

    #[test]
    fn config_runtime_dir_reads_core_section() {
        let dir = scratch_root().join(format!("pii-uds-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("pii.toml");
        std::fs::write(&cfg, "[core]\nruntime_dir = \"/tmp/pii-runtime\"\n").unwrap();
        let got = super::config_runtime_dir(&cfg);
        assert_eq!(got, Some(std::path::PathBuf::from("/tmp/pii-runtime")));
    }

    #[test]
    fn config_runtime_dir_none_when_absent() {
        let dir = scratch_root().join(format!("pii-uds-none-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("pii.toml");
        std::fs::write(&cfg, "[core]\nmodel = \"x\"\n").unwrap();
        let got = super::config_runtime_dir(&cfg);
        assert_eq!(got, None);
    }
}
