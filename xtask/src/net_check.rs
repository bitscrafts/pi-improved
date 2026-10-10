//! `xtask net-check` (F2.13/F2.14): the load-bearing Phase 3 gate.
//!
//! `pii-net` (plus the two sanctioned control-socket files) is the only
//! place in the workspace that may open a socket or resolve a name. Tests
//! are in scope too; `pii_net::testing` fakes exist so they never need one.

use std::path::Path;

/// Socket/name-resolution symbols that may only appear in the allowlist.
///
/// Each entry is `concat!`-split so the scanner's own table does not
/// literally contain the symbol it looks for (the gate scans itself too).
pub const SYMBOLS: &[&str] = &[
    concat!("TcpListener", "::bind"),
    concat!("TcpStream", "::connect"),
    concat!("UdpSocket", "::bind"),
    concat!("UdpSocket", "::connect"),
    concat!("UnixListener", "::bind"),
    concat!("UnixStream", "::connect"),
    concat!("to_socket", "_addrs"),
    concat!("ToSocket", "Addrs"),
    concat!("lookup", "_host"),
];

/// The sanctioned call sites (spec F2.13): `pii-net`'s three socket files
/// plus the pre-existing control-plane UDS files.
pub const ALLOWLIST: &[&str] = &[
    "crates/pii-net/src/dial.rs",
    "crates/pii-net/src/listen.rs",
    "crates/pii-net/src/resolve.rs",
    // P4.7 (spec 22): service.rs is inside pii-net (the sanctioned socket
    // crate); its production path uses TcpStream via dial.rs/Conn, and the
    // only raw loopback-bind in a test is the streaming dechunk fake.
    "crates/pii-net/src/service.rs",
    "crates/pii-server/src/engine/uds.rs",
    // B2 (011 A.11): client-side connect moved to pii-uds so clients do not
    // pull the daemon tree; the sanctioned client connect site lives here.
    "crates/pii-uds/src/lib.rs",
    "crates/pii-cli/src/cmd_daemon.rs",
    // P4.6 (spec 19): pii-web binds a loopback TCP listener for the viewer.
    "crates/pii-web/src/main.rs",
    // Spec 117: the in-process loopback MQTT broker the tests bring with them
    // (no system broker exists on the CI host); it binds 127.0.0.1 only and
    // is `#[path]`-included by pii-server's MQTT end-to-end test.
    "crates/pii-channels/tests/mqtt_broker/mod.rs",
];

/// Scans one in-memory file; returns `NET-CHECK FAIL:` violation lines.
/// Split out for F2.14: the gate is proven to fail when violated.
pub fn scan_one(path: &str, contents: &str, allowlist: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if allowlist.contains(&path) {
        return out;
    }
    for (idx, line) in contents.lines().enumerate() {
        let Some(symbol) = SYMBOLS.iter().find(|s| line.contains(*s)) else {
            continue;
        };
        out.push(format!(
            "NET-CHECK FAIL: {path}:{} — {symbol} outside pii-net",
            idx + 1
        ));
    }
    out
}

/// Walks `crates/*/src/**/*.rs` and `xtask/src/**/*.rs`; every hit must be
/// allowlisted. Violations print and the gate exits non-zero.
///
/// # Errors
/// One violation line per offending symbol occurrence.
pub fn run() -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    for base in ["crates", "xtask"] {
        walk_rs(Path::new(base), &mut |path, contents| {
            violations.extend(scan_one(path, contents, ALLOWLIST));
        });
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Depth-first `.rs` walk, deterministic order, no third-party walker.
fn walk_rs(dir: &Path, visit: &mut dyn FnMut(&str, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<std::path::PathBuf> =
        entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk_rs(&p, visit);
        } else if p.extension().is_some_and(|e| e == "rs")
            && let Ok(contents) = std::fs::read_to_string(&p)
        {
            visit(&p.to_string_lossy(), &contents);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{ALLOWLIST, scan_one};

    /// An allowlisted path with the same content is clean.
    #[test]
    fn net_check_allows_sanctioned_files() {
        let src = concat!(
            "let s = std::net::TcpStream",
            "::connect_timeout(addr, d);\n"
        );
        assert!(scan_one("crates/pii-net/src/dial.rs", src, ALLOWLIST).is_empty());
    }
}
