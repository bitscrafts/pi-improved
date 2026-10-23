//! Spec 38 TDD table: basic GET, https, POST blocked, headers blocked —
//! all through the `pii-net` chokepoint with audit logging. The fake HTTP
//! server (`pii_net::testing`) keeps tests off the real network.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_http_fetch::audit::AuditLog;
use pii_http_fetch::{HttpFetchTool, Tool};
use pii_net::grant::{NetGrant, StaticGrants};
use pii_net::resolve::StaticResolver;
use pii_net::{NetConfig, NetService};
use serde_json::json;
use std::sync::Arc;

fn net_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

/// A client `NetService` pointed at the fake, with an open local grant —
/// the same shape production uses, but loopback-only.
fn client_for(fake: &pii_net::testing::FakeHttpServer) -> NetService {
    let mut resolver = StaticResolver::new();
    resolver.insert("fake.local", fake.addr.port(), vec![fake.addr]);
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", NetGrant::open_local());
    grants.insert("*", "http:fake.local", NetGrant::open_local());
    NetService::new(NetConfig {
        resolver: Arc::new(resolver),
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    })
}

fn open_local_grant_service() -> NetService {
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", NetGrant::open_local());
    NetService::new(NetConfig {
        resolver: Arc::new(StaticResolver::new()),
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    })
}

/// Snapshot helper: clones the audit buffer, dropping the lock before any
/// further work (the nursery gate forbids holding the lock across `stop`).
fn audit_snapshot(buf: &parking_lot::Mutex<Vec<String>>) -> Vec<String> {
    buf.lock().clone()
}

/// `fetch_basic_get`: a plain GET returns the body and lands in the audit log.
#[test]
fn fetch_basic_get() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"GET","path_prefix":"/","status":200,"body":{"html":"<html>hello</html>"}}]}"#,
    )
    .unwrap();
    let (log, buf) = AuditLog::in_memory();
    let tool = HttpFetchTool::new(client_for(&fake), log);
    let out = tool
        .call(
            &net_ctx(),
            &json!({"url": format!("http://fake.local:{}/", fake.addr.port())}),
        )
        .unwrap();
    assert_eq!(out["status"], 200);
    let body = out["bytes_in"].as_u64().unwrap();
    assert!(body > 0, "body must be non-empty");
    let lines = audit_snapshot(&buf);
    assert_eq!(lines.len(), 1);
    assert!(
        lines
            .first()
            .unwrap()
            .starts_with("FEED: GET http://fake.local:"),
        "audit line: {}",
        lines.first().unwrap()
    );
    assert!(
        lines.first().unwrap().contains("method=GET"),
        "audit: {}",
        lines.first().unwrap()
    );
    fake.stop();
}

/// `fetch_https`: https is accepted and routed through the same chokepoint.
/// Against the plaintext fake, TLS negotiation fails — which proves the
/// https scheme was accepted by the tool and passed to the chokepoint
/// (scheme handling lives in pii-net, spec 17). A malformed-URL refusal
/// would never reach the transport.
#[test]
fn fetch_https_routes_through_chokepoint() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"GET","path_prefix":"/","status":200,"body":{"ok":true}}]}"#,
    )
    .unwrap();
    let (log, buf) = AuditLog::in_memory();
    let tool = HttpFetchTool::new(client_for(&fake), log);
    let out = tool.call(
        &net_ctx(),
        &json!({"url": format!("https://fake.local:{}/", fake.addr.port()), "timeout_ms": 2_000}),
    );
    // The TLS handshake against a plaintext server fails: Unavailable from
    // the chokepoint, audited as DENIED. An Invalid error here would mean
    // the tool refused https itself — that is NOT allowed (spec 17 permits
    // https end-to-end).
    let err = out.unwrap_err();
    assert!(
        !matches!(err, pii_abi::PiiError::Invalid(_)),
        "https must be accepted by the tool, got {err:?}"
    );
    let lines = audit_snapshot(&buf);
    assert_eq!(lines.len(), 1);
    assert!(
        lines
            .first()
            .unwrap()
            .starts_with("FEED: DENIED https://fake.local:"),
        "{}",
        lines.first().unwrap()
    );
    fake.stop();
}

/// `fetch_post_blocked`: any non-GET method is refused before the chokepoint.
#[test]
fn fetch_post_blocked() {
    let fake = pii_net::testing::FakeHttpServer::start(r#"{"routes":[]}"#).unwrap();
    let (log, buf) = AuditLog::in_memory();
    let tool = HttpFetchTool::new(client_for(&fake), log);
    for method in ["POST", "PUT", "DELETE", "PATCH"] {
        let err = tool
            .call(
                &net_ctx(),
                &json!({"url": "http://fake.local/", "method": method}),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("only GET is permitted"),
            "{method}: {err}"
        );
    }
    // Nothing reached the chokepoint and the refusals are audited.
    assert_eq!(
        fake.requests(),
        [] as [pii_net::testing::RecordedRequest; 0]
    );
    let lines = audit_snapshot(&buf);
    assert_eq!(lines.len(), 4, "each refusal is audited: {lines:?}");
    assert!(lines.iter().all(|l| l.starts_with("FEED: DENIED ")));
    fake.stop();
}

/// `fetch_with_headers`: caller headers are refused — credential-free API.
#[test]
fn fetch_with_headers_blocked() {
    let fake = pii_net::testing::FakeHttpServer::start(r#"{"routes":[]}"#).unwrap();
    let (log, buf) = AuditLog::in_memory();
    let tool = HttpFetchTool::new(client_for(&fake), log);
    for key in ["headers", "auth", "cookies"] {
        let err = tool
            .call(
                &net_ctx(),
                &json!({"url": "http://fake.local/", key: {"Authorization": "Bearer x"}}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("blocked"), "{key}: {err}");
    }
    assert!(fake.requests().is_empty(), "no request may leave the tool");
    let lines = audit_snapshot(&buf);
    assert_eq!(lines.len(), 3);
    fake.stop();
}

/// A file-backed audit log matches the in-memory bytes byte-for-byte
/// (determinism across sinks).
#[test]
fn audit_file_sink_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("audit.log");
    let (mem_log, buf) = AuditLog::in_memory();
    let file_log = AuditLog::to_file(path.clone());
    let line =
        "FEED: GET http://x/ -> 200-OK status=200 bytes_in=3 headers=none method=GET body=none";
    mem_log.append(line).unwrap();
    file_log.append(line).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, format!("{line}\n"));
    assert_eq!(buf.lock().first().map(String::as_str), Some(line));
}

/// A service with NO grant for the fetch key is refused by the chokepoint
/// and audited as DENIED — the chokepoint remains the enforcement point.
#[test]
fn fetch_without_grant_denied_by_chokepoint() {
    let fake = pii_net::testing::FakeHttpServer::start(r#"{"routes":[]}"#).unwrap();
    let (log, buf) = AuditLog::in_memory();
    let tool = HttpFetchTool::new(open_local_grant_service(), log);
    let err = tool
        .call(
            &net_ctx(),
            &json!({"url": format!("http://fake.local:{}/", fake.addr.port()), "timeout_ms": 2_000}),
        )
        .unwrap_err();
    assert!(
        matches!(err, pii_abi::PiiError::Denied(_)),
        "chokepoint must deny without grant, got {err:?}"
    );
    let lines = audit_snapshot(&buf);
    assert_eq!(lines.len(), 1);
    assert!(
        lines
            .first()
            .unwrap()
            .starts_with("FEED: DENIED http://fake.local:")
    );
    fake.stop();
}
