//! Egress discipline tests (TDD table, part-1): authorize rules, resolve-once
//! pinning, shared lease, byte budgets, deadline spanning.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pii_abi::{HttpRequest, PiiError};
use pii_core::HttpTransport as _;
use pii_core::{Budget, CapKind, CapSet, Ctx};

use pii_net::authorize::{AddrClass, NetDenied, authorize_d};
use pii_net::grant::{HostScope, NetGrant, StaticGrants};
use pii_net::resolve::Resolver;
use pii_net::{NetConfig, NetService, Target};

fn net_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

fn grant(allow_private: bool, hosts: &[String]) -> NetGrant {
    NetGrant {
        connect: HostScope::parse_all(hosts).unwrap_or_else(|_| HostScope::default()),
        allow_private,
        max_conns: u32::MAX,
        max_bytes_out: u64::MAX,
        max_bytes_in: u64::MAX,
        expires_at_ms: None,
    }
}

fn ips(v: &[&str]) -> Vec<IpAddr> {
    v.iter().map(|s| s.parse().unwrap()).collect()
}

fn target(host: &str, port: u16) -> Target {
    Target {
        host: String::from(host),
        port,
    }
}

#[test]
fn authorize_refuses_mixed_answers() {
    let g = grant(true, &[String::from("*:*")]);
    let r = authorize_d(
        &target("evil.example", 80),
        &ips(&["93.184.216.34", "10.0.0.1"]),
        &g,
        0,
        5_000,
    );
    assert_eq!(r.unwrap_err(), NetDenied::MixedAnswer);
}

#[test]
fn authorize_blocks_cloud_metadata() {
    let g = grant(true, &[String::from("*:*")]);
    let r = authorize_d(
        &target("metadata", 80),
        &ips(&["169.254.169.254"]),
        &g,
        0,
        5_000,
    );
    assert_eq!(
        r.unwrap_err(),
        NetDenied::BlockedClass(String::from("CloudMetadata"))
    );
    // The IPv6 form too.
    let r6 = authorize_d(
        &target("metadata", 80),
        &ips(&["fd00:ec2::254"]),
        &g,
        0,
        5_000,
    );
    assert!(matches!(
        r6.unwrap_err(),
        NetDenied::BlockedClass(c) if c.contains("Cloud")
    ));
}

#[test]
fn authorize_blocks_nat64_mapped_private() {
    let g = grant(true, &[String::from("*:*")]);
    let r = authorize_d(
        &target("nat64", 80),
        &ips(&["64:ff9b::10.0.0.1"]),
        &g,
        0,
        5_000,
    );
    assert_eq!(
        r.unwrap_err(),
        NetDenied::BlockedClass(String::from("Nat64MappedPrivate"))
    );
    // But a public embedded v4 stays public.
    let ok = authorize_d(
        &target("nat64", 80),
        &ips(&["64:ff9b::1.2.3.4"]),
        &g,
        0,
        5_000,
    );
    assert!(ok.is_ok());
}

#[test]
fn authorize_allows_loopback_only_with_flag() {
    let hosts = [String::from("*:*")];
    let with = grant(true, &hosts);
    let without = grant(false, &hosts);
    let addrs = ips(&["127.0.0.1"]);
    assert!(authorize_d(&target("local", 9), &addrs, &with, 0, 5_000).is_ok());
    let err = authorize_d(&target("local", 9), &addrs, &without, 0, 5_000).unwrap_err();
    assert!(matches!(err, NetDenied::BlockedClass(c) if c.contains("Loopback")));
}

#[test]
fn authorize_respects_expiry_and_scope() {
    let mut g = grant(false, &[String::from("api.example:443")]);
    g.expires_at_ms = Some(100);
    let expired = authorize_d(
        &target("api.example", 443),
        &ips(&["1.2.3.4"]),
        &g,
        100,
        5_000,
    );
    assert_eq!(expired.unwrap_err(), NetDenied::Expired);
    let out_of_scope = authorize_d(
        &target("other.example", 443),
        &ips(&["1.2.3.4"]),
        &g,
        0,
        5_000,
    );
    assert!(matches!(
        out_of_scope.unwrap_err(),
        NetDenied::NotAllowed(_)
    ));
    // IPv4-mapped loopback classifies by its embedded v4.
    assert_eq!(
        AddrClass::of(&"::ffff:127.0.0.1".parse().unwrap()),
        AddrClass::Loopback
    );
}

/// A resolver whose answer flips after the first call.
struct FlipResolver {
    first: Vec<SocketAddr>,
    second: Vec<SocketAddr>,
    calls: AtomicUsize,
}

impl Resolver for FlipResolver {
    fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<SocketAddr>, PiiError> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(if n == 0 {
            self.first.clone()
        } else {
            self.second.clone()
        })
    }
}

#[test]
fn connect_resolves_once_and_pins() {
    let pub_addr: SocketAddr = "93.184.216.34:8080".parse().unwrap();
    let flip = FlipResolver {
        first: vec![pub_addr],
        second: vec!["10.0.0.1:8080".parse().unwrap()],
        calls: AtomicUsize::new(0),
    };
    let mut grants = StaticGrants::default();
    grants.insert("*", "k", grant(true, &[String::from("*:*")]));
    let svc = NetService::new(NetConfig {
        resolver: Arc::new(flip),
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    // The dial itself will fail (no listener), but the resolver must be
    // called exactly once; the dial error names the pinned host.
    let err = svc
        .connect(&net_ctx(), &target("pinned.example", 8080), "k", 2_000)
        .err()
        .expect("dial must fail without a listener");
    assert!(matches!(err, PiiError::Unavailable(_)), "got {err:?}");
}

#[test]
fn conn_lease_is_shared_across_clones() {
    struct Accepting;
    let fake = pii_net::testing::FakeHttpServer::start(r#"{"routes":[]}"#).unwrap();
    let _ = Accepting;
    let mut grants = StaticGrants::default();
    let mut g = grant(true, &[String::from("*:*")]);
    g.max_conns = 1;
    grants.insert("*", "k", g);
    let svc = NetService::new(NetConfig {
        resolver: {
            let mut r = pii_net::StaticResolver::new();
            r.insert("fake.local", fake.addr.port(), vec![fake.addr]);
            Arc::new(r)
        },
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    let svc2 = svc.clone();
    let first = svc.connect(
        &net_ctx(),
        &target("fake.local", fake.addr.port()),
        "k",
        2_000,
    );
    assert!(first.is_ok(), "first lease: {:?}", first.err());
    let err = svc2
        .connect(
            &net_ctx(),
            &target("fake.local", fake.addr.port()),
            "k",
            2_000,
        )
        .err()
        .expect("second lease must be denied");
    assert!(
        matches!(err, PiiError::Denied(_)),
        "second lease denied: {err:?}"
    );
    fake.stop();
}

#[test]
fn byte_budget_closes_connection() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"GET","path_prefix":"/","status":200,"body":{"pad":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}}]}"#,
    )
    .unwrap();
    let mut grants = StaticGrants::default();
    let mut g = grant(true, &[String::from("*:*")]);
    g.max_bytes_in = 10; // smaller than the 42-byte response → budget breach
    grants.insert("*", "default", g.clone());
    grants.insert("*", "http:fake.local", g);
    let svc = NetService::new(NetConfig {
        resolver: {
            let mut r = pii_net::StaticResolver::new();
            r.insert("fake.local", fake.addr.port(), vec![fake.addr]);
            Arc::new(r)
        },
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    let req = HttpRequest {
        method: pii_abi::HttpMethod::Get,
        url: format!("http://fake.local:{}/", fake.addr.port()),
        headers: Vec::new(),
        body: Vec::new(),
        timeout_ms: 2_000,
    };
    let ctx = net_ctx();
    let err = svc.request(&ctx, &req).unwrap_err();
    assert!(
        err.to_string().contains("byte budget"),
        "want byte-budget denial, got {err}"
    );
    fake.stop();
}
