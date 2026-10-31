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
    // Spec 118: the file was split into service/{mod,conn,http,dechunk}.rs
    // (629ac9f) and this entry went stale; the only socket call is the same
    // test-only loopback fake in service/mod.rs.
    "crates/pii-net/src/service.rs",
    "crates/pii-net/src/service/mod.rs",
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
    // Spec 118: sockets that were already in the tree when this gate was first
    // run (it ran nowhere before 118, so they were never reviewed). Test files
    // bind/connect loopback or tempdir sockets to fake a daemon or server.
    "crates/pii-cli/tests/transport_failures_visible.rs",
    "crates/pii-cli/tests/tui_failure_logged.rs",
    "crates/pii-evolve/tests/launcher.rs",
    "crates/pii-net/tests/accept_pool.rs",
    // Test-only loopback fakes that serve a real TCP/TLS peer so a close (or a
    // refusal) can be INJECTED at the socket; the net calls under test stay
    // inside pii-net. Same sanctioned class as spec 118's test files: the
    // close_notify, byte-budget and Discord adapter tests landed after 118
    // without a fresh gate run, which is why they were never reviewed here.
    "crates/pii-channels/tests/discord.rs",
    "crates/pii-net/src/service/conn_close_tests.rs",
    "crates/pii-net/tests/budget_survives_a_day.rs",
    "crates/pii-net/tests/budget_window.rs",
    "crates/pii-net/tests/stream_close_is_eof.rs",
    // Spec 200: the provider-level close semantis test serves its own TLS loopback
    // peer (rustls ServerConfig + TcpListener bind) so close_notify can be
    // injected at the socket; pii-net's same-named test covers the transport
    // half. Same sanctioned test-fake class as spec 118's entries.
    "crates/pii-model/tests/stream_close_is_eof.rs",
    // Spec 230: the coder's container runtime talks HTTP/1.1 over the podman
    // API socket — the one UnixStream::connect site outside pii-uds (rootless
    // podman cannot be driven by a child process under the sandbox; measured
    // M0 PASS through this socket). Sync std stream with a read-timeout tick
    // a cancelled run can stop, which the async client_connect does not offer.
    "crates/pii-server/src/node_kinds/nodes/coder_runtime.rs",
    // Spec 241 moved that same client (unchanged purpose) into its own file when `coder_runtime.rs` reached the 450-line gate.
    "crates/pii-server/src/node_kinds/nodes/coder_runtime_api.rs",
    // Spec 230: the test's fake podman serves the REST paths on a bound UDS.
    "crates/pii-server/tests/coder_starts_through_the_runtime_socket.rs",
    // Spec 241: the same sanctioned fake, extended to serve `logs`/`stop` and hang on demand.
    "crates/pii-server/tests/coder_container_bounded.rs",
    // Spec 252: the same sanctioned fake again, serving the inspect reply whose digest the runtime must never journal as a bare prefix.
    "crates/pii-server/tests/image_digest_is_never_a_bare_prefix.rs",
    // Spec 263: the same sanctioned fake again, serving a framed container log so the container path's head-and-tail capture is driven through the real runtime client.
    "crates/pii-server/tests/coder_output_keeps_the_tail.rs",
    // Spec 325: health must see a daemon that answers with a FOREIGN build revision, which no real daemon in a
    // test can be. This test therefore serves its own control.sock stub answering `sys.hello` — the same
    // sanctioned test-fake class as the entries above, injecting the one field the rule compares.
    "crates/pii-cli/tests/health_drift.rs",
];

/// Scans one in-memory file; returns `NET-CHECK FAIL:` violation lines.
/// Split out for F2.14: the gate is proven to fail when violated.
pub fn scan_one(path: &str, contents: &str, allowlist: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    if allowlist.contains(&path) {
        return out;
    }
    let code = strip_comments(contents);
    for (idx, line) in code.lines().enumerate() {
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

/// Blanks line, block (nested) and doc comments with spaces, keeping every
/// newline so reported line numbers stay true (spec 118: a doc comment that
/// merely names a symbol is not a call). String and char literals are kept
/// verbatim and skipped over, so `"//"` cannot open a comment and hide code.
fn strip_comments(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let (mut i, mut depth) = (0, 0usize);
    while let Some(&cur) = c.get(i) {
        let pair = (cur, c.get(i + 1).copied());
        if depth > 0 {
            match pair {
                ('/', Some('*')) => {
                    depth += 1;
                    i += 2;
                }
                ('*', Some('/')) => {
                    depth -= 1;
                    i += 2;
                }
                _ => {
                    out.push(if cur == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
        } else if pair == ('/', Some('*')) {
            depth = 1;
            i += 2;
        } else if pair == ('/', Some('/')) {
            while c.get(i).is_some_and(|&h| h != '\n') {
                i += 1;
            }
        } else if let Some(end) = literal_end(&c, i) {
            out.extend(c.get(i..end).unwrap_or_default());
            i = end;
        } else {
            out.push(cur);
            i += 1;
        }
    }
    out
}

/// Count of consecutive `#` from `from`.
fn hashes_at(c: &[char], from: usize) -> usize {
    c.get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|&&h| h == '#')
        .count()
}

/// If a string, raw string or char literal starts at `i`, the index just past it.
fn literal_end(c: &[char], i: usize) -> Option<usize> {
    let at = |j: usize| c.get(j).copied();
    let prev_is_ident = i > 0
        && c.get(i - 1)
            .is_some_and(|p| p.is_alphanumeric() || *p == '_');
    match at(i)? {
        '"' => Some(quoted_end(c, i + 1)),
        'r' | 'b' if !prev_is_ident => {
            let mut j = i + usize::from(at(i) == Some('b'));
            if at(j) == Some('r') {
                j += 1;
                let hashes = hashes_at(c, j);
                j += hashes;
                if at(j) != Some('"') {
                    return None;
                }
                j += 1;
                while j < c.len() {
                    if at(j) == Some('"') && hashes_at(c, j + 1) >= hashes {
                        return Some(j + 1 + hashes);
                    }
                    j += 1;
                }
                Some(c.len())
            } else if at(i) == Some('b') && at(j) == Some('"') {
                Some(quoted_end(c, j + 1))
            } else {
                None
            }
        }
        '\'' => {
            // Char literal `'x'` / `'\n'`; otherwise a lifetime.
            if at(i + 1) == Some('\\') {
                let close = c.get(i + 2..)?.iter().position(|&h| h == '\'')?;
                Some(i + 2 + close + 1)
            } else if at(i + 2) == Some('\'') {
                Some(i + 3)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Index just past the closing quote of a string whose body starts at `from`.
fn quoted_end(c: &[char], from: usize) -> usize {
    let mut j = from;
    while let Some(&h) = c.get(j) {
        match h {
            '\\' => j += 2,
            '"' => return j + 1,
            _ => j += 1,
        }
    }
    c.len()
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

    const SYM: &str = concat!("UnixStream", "::connect");

    fn hits(src: &str) -> usize {
        scan_one("crates/pii-x/src/a.rs", src, ALLOWLIST).len()
    }

    /// Spec 118 R1: a symbol named inside any comment form is not a call.
    #[test]
    fn net_check_ignores_comments() {
        assert_eq!(
            hits(&format!("/// docs mention {SYM} here\nfn f() {{}}\n")),
            0
        );
        assert_eq!(hits(&format!("//! inner doc {SYM}\n")), 0);
        assert_eq!(hits(&format!("// plain {SYM}\n")), 0);
        assert_eq!(hits(&format!("/* block {SYM} */\n")), 0);
        assert_eq!(hits(&format!("/* a\n {SYM}\n b */\nfn f() {{}}\n")), 0);
        assert_eq!(hits(&format!("/* outer /* nested */ {SYM} */\n")), 0);
    }

    /// Spec 118 R1 control: code next to a comment is still code.
    #[test]
    fn net_check_still_flags_code_beside_comments() {
        assert_eq!(hits(&format!("let s = {SYM}(p); // ok\n")), 1);
        assert_eq!(hits(&format!("/* x */ let s = {SYM}(p);\n")), 1);
        assert_eq!(hits(&format!("/* a\n b */ let s = {SYM}(p);\n")), 1);
        assert_eq!(hits(&format!("let u = \"//\"; let s = {SYM}(p);\n")), 1);
        assert_eq!(hits(&format!("let c = '\"'; let s = {SYM}(p);\n")), 1);
        assert_eq!(hits(&format!("let s = {SYM}(p);\n")), 1);
        assert_eq!(hits(&format!("/// doc\nlet s = {SYM}(p);\n")), 1);
    }

    /// An allowlisted path with the same content is clean.
    #[test]
    fn net_check_allows_sanctioned_files() {
        let src = concat!(
            "let s = std::net::TcpStream",
            "::connect_timeout(addr, d);\n"
        );
        assert_eq!(
            scan_one("crates/pii-net/src/dial.rs", src, ALLOWLIST),
            [] as [String; 0]
        );
    }

    /// Spec 130 R5: `pii-evolve`'s production code holds no allowlist entry, so
    /// a direct dial there is a violation that names the file.
    #[test]
    fn net_check_refuses_a_direct_dial_in_pii_evolve() {
        let src = format!("let s = {SYM}(p);\n");
        let file = ["crates/pii-evolve/src", "driver.rs"].join("/");
        let out = scan_one(&file, &src, ALLOWLIST);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out.first().is_some_and(|l| l.contains(&file)));
    }
}
