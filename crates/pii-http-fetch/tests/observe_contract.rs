//! Spec 101 R3: fetch outcomes and allowlist denials reach a `pii-observe`
//! observer by their registered names.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_http_fetch::audit::AuditLog;
use pii_http_fetch::{HttpFetchTool, Tool};
use pii_net::grant::{NetGrant, StaticGrants};
use pii_net::resolve::StaticResolver;
use pii_net::{NetConfig, NetService};
use pii_observe::{Recorder, names};
use serde_json::json;

fn net_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

#[test]
fn allowlist_denial_is_reported_by_name() {
    // No `http:<host>` grant and no default: the chokepoint refuses.
    let svc = NetService::new(NetConfig {
        resolver: Arc::new(StaticResolver::new()),
        grants: Arc::new(StaticGrants::default()),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    let (log, _buf) = AuditLog::in_memory();
    let rec = Arc::new(Recorder::new());
    let tool = HttpFetchTool::new(svc, log).with_observer(rec.clone());
    let err = tool
        .call(&net_ctx(), &json!({"url": "http://nowhere.local:9/"}))
        .unwrap_err();
    assert!(matches!(err, pii_abi::PiiError::Denied(_)), "got {err:?}");
    let denied = rec.events_named(names::FETCH_ALLOWLIST_DENIED);
    assert_eq!(denied.len(), 1, "{:?}", rec.events());
    assert_eq!(
        denied[0].fields.get("host"),
        Some(&json!("nowhere.local")),
        "the denial names the host, never the URL query or any secret"
    );
}

#[test]
fn successful_fetch_reports_outcome() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"GET","path_prefix":"/","status":200,"body":{"a":1}}]}"#,
    )
    .unwrap();
    let mut resolver = StaticResolver::new();
    resolver.insert("fake.local", fake.addr.port(), vec![fake.addr]);
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", NetGrant::open_local());
    grants.insert("*", "http:fake.local", NetGrant::open_local());
    let svc = NetService::new(NetConfig {
        resolver: Arc::new(resolver),
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    let (log, _buf) = AuditLog::in_memory();
    let rec = Arc::new(Recorder::new());
    let tool = HttpFetchTool::new(svc, log).with_observer(rec.clone());
    tool.call(
        &net_ctx(),
        &json!({"url": format!("http://fake.local:{}/", fake.addr.port())}),
    )
    .unwrap();
    let ev = rec.events_named(names::FETCH_OUTCOME);
    assert_eq!(ev.len(), 1, "{:?}", rec.events());
    assert_eq!(ev[0].fields.get("status"), Some(&json!(200)));
    assert!(rec.events_named(names::FETCH_ALLOWLIST_DENIED).is_empty());
}
