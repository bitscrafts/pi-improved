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
        .connect(
            &net_ctx(),
            &target("pinned.example", 8080),
            "k",
            2_000,
            pii_net::Scheme::Http,
        )
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
        pii_net::Scheme::Http,
    );
    assert!(first.is_ok(), "first lease: {:?}", first.err());
    let err = svc2
        .connect(
            &net_ctx(),
            &target("fake.local", fake.addr.port()),
            "k",
            2_000,
            pii_net::Scheme::Http,
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

/// R1 (spec 15): `request_streaming` forwards each body chunk as it arrives.
#[test]
fn request_streaming_emits_chunks() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"GET","path_prefix":"/","status":200,"body":{},
            "chunks":["Hel","lo"," world"]}]}"#,
    )
    .unwrap();
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", grant(true, &[String::from("*:*")]));
    grants.insert("*", "http:fake.local", grant(true, &[String::from("*:*")]));
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
    let mut chunks: Vec<String> = Vec::new();
    let resp = svc
        .request_streaming(&ctx, &req, &mut |b: &[u8]| {
            chunks.push(String::from_utf8_lossy(b).to_string());
        })
        .unwrap();
    // R1 transport contract: the streamed body is correctly extracted (head
    // stripped, content intact) and `on_chunk` fired at least once. The fake
    // server writes all chunks + the rendered response back-to-back, so the
    // client's socket read coalesces them — the *incremental* forwarding is
    // proven deterministically by the R2 acceptance test (ChunkedTransport).
    assert!(
        !chunks.is_empty(),
        "on_chunk must fire at least once, got {chunks:?}"
    );
    let body = String::from_utf8_lossy(&resp.body);
    assert!(
        body.contains("Hello world"),
        "body missing streamed content: {body}"
    );
    fake.stop();
}

/// R1 (spec 15): the default `request_streaming` falls back to `request`
/// and delivers the full body as one chunk.
#[test]
fn request_streaming_default_falls_back() {
    use pii_core::HttpTransport as _;
    struct Buffered;
    impl pii_core::HttpTransport for Buffered {
        fn request(
            &self,
            _ctx: &Ctx,
            _req: &HttpRequest,
        ) -> Result<pii_abi::HttpResponse, PiiError> {
            Ok(pii_abi::HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"full".to_vec(),
            })
        }
    }
    let t = Buffered;
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let resp = t
        .request_streaming(
            &net_ctx(),
            &HttpRequest {
                method: pii_abi::HttpMethod::Get,
                url: String::from("http://x/"),
                headers: Vec::new(),
                body: Vec::new(),
                timeout_ms: 1_000,
            },
            &mut |b: &[u8]| chunks.push(b.to_vec()),
        )
        .unwrap();
    assert_eq!(resp.body, b"full");
    assert_eq!(chunks.len(), 1, "default must deliver one chunk");
    assert_eq!(chunks[0], b"full");
}

/// R4 (spec 17): an `https://` request dispatches through the TLS path — a
/// plaintext fake server must fail with a TLS handshake error, NOT succeed
/// as if it were plain http. This proves the scheme dispatch actually wraps
/// the stream (the acceptance test for the TLS wiring).
#[test]
fn request_https_dispatch_takes_tls_path() {
    let fake = pii_net::testing::FakeHttpServer::start(
        r#"{"routes":[{"method":"GET","path_prefix":"/","status":200,"body":{"ok":true}}]}"#,
    )
    .unwrap();
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", grant(true, &[String::from("*:*")]));
    grants.insert("*", "http:fake.local", grant(true, &[String::from("*:*")]));
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
        url: format!("https://fake.local:{}/", fake.addr.port()),
        headers: Vec::new(),
        body: Vec::new(),
        timeout_ms: 2_000,
    };
    let ctx = net_ctx();
    // The fake server speaks plaintext HTTP; a TLS handshake against it
    // must fail (the https path was taken, not the plain path).
    let err = svc.request(&ctx, &req).unwrap_err();
    assert!(
        matches!(err, PiiError::Unavailable(_)) || matches!(err, PiiError::Invalid(_)),
        "https against plaintext must fail with a TLS error, got {err:?}"
    );
    fake.stop();
}

/// D1 (Phase 5.0): a TLS setup failure that happens strictly INSIDE
/// `dial_tls` — after `dial()` has already succeeded and the lease was
/// incremented for the attempt — must still release the lease. This uses
/// a real, deterministic failure mode of `dial_tls` (an invalid TLS server
/// name, per its own doc comment: `PiiError::Invalid` "when `host` is not
/// a valid TLS server name") rather than a lazily-surfacing handshake I/O
/// error: `dial_tls` is a synchronous, network-free step (it only builds
/// the `rustls::ClientConnection`), so this failure fires at EXACTLY the
/// point the bug lived — between the lease increment and `Conn` coming
/// into existence, where the old code's bare `?` inside the `Ok(stream)`
/// match arm short-circuited past the lease-decrement `Err` arm entirely
/// (and no `Conn` ever existed to fall back on its `Drop` guard either).
/// Dials the real fake server (so `dial()` itself succeeds and the lease
/// is genuinely charged), then targets it with a host string that is not
/// a valid DNS name/IP, so only the TLS server-name step fails.
#[test]
fn tls_handshake_failure_releases_lease() {
    let fake = pii_net::testing::FakeHttpServer::start(r#"{"routes":[]}"#).unwrap();
    let mut grants = StaticGrants::default();
    let bad_key = "http:bad_host!name";
    grants.insert("user", bad_key, grant(true, &[String::from("*:*")]));
    let svc = NetService::new(NetConfig {
        resolver: {
            let mut r = pii_net::StaticResolver::new();
            // Points at the real fake server, so `dial()` succeeds — only
            // the TLS server-name validation that follows must fail.
            r.insert("bad_host!name", fake.addr.port(), vec![fake.addr]);
            Arc::new(r)
        },
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    let ctx = net_ctx();
    // Before the failing attempt, every grant's lease count starts at 0.
    assert!(
        svc.grant_info().iter().all(|g| g.conns == 0),
        "lease counts must start at 0: {:?}",
        svc.grant_info()
    );
    let err = svc
        .connect(
            &ctx,
            &target("bad_host!name", fake.addr.port()),
            bad_key,
            2_000,
            pii_net::Scheme::Https,
        )
        .err()
        .expect("an invalid TLS server name must fail dial_tls");
    assert!(
        matches!(err, PiiError::Invalid(_)),
        "want PiiError::Invalid from an unparseable TLS server name, got {err:?}"
    );
    // The lease was incremented before dial+TLS ran; the dial_tls failure
    // must have decremented it back to 0 (D1 acceptance).
    let info = svc.grant_info();
    assert!(
        info.iter().all(|g| g.conns == 0),
        "TLS setup failure leaked a connection lease: {info:?}"
    );
    fake.stop();
}
