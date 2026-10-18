//! The byte budget survives the day (spec 180): the window belongs to ONE
//! connection, so neither earlier requests nor a concurrent connection on the
//! same grant can spend it, and one response larger than the window is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the test serves its own loopback responder (the stock fake
    // server sleeps 20 ms per accept); grants do not apply to the test's own
    // listener, as in accept_pool.rs.
    clippy::disallowed_methods
)]

use std::sync::Arc;

use pii_abi::HttpRequest;
use pii_core::HttpTransport as _;
use pii_core::{Budget, CapKind, CapSet, Ctx};

use pii_net::grant::{HostScope, NetGrant, StaticGrants};
use pii_net::{NetConfig, NetService, Scheme, Target};

const WINDOW: u64 = 1024 * 1024;
const POLLS: usize = 5_000;

fn net_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

/// A blocking-accept responder: the stock fake server's accept loop sleeps 20 ms
/// between connections, which would make 50,000 sequential polls take ~17 min.
struct Responder {
    addr: std::net::SocketAddr,
}

fn responder(body: &str, connections: usize) -> Responder {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match std::io::Read::read(&mut s, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            let _ = std::io::Write::write_all(&mut s, resp.as_bytes());
        }
    });
    Responder { addr }
}

fn service(fake: &Responder, window: u64) -> NetService {
    let g = NetGrant {
        connect: HostScope::parse_all(&[String::from("*:*")]).unwrap(),
        allow_private: true,
        max_conns: u32::MAX,
        max_bytes_out: window,
        max_bytes_in: window,
        expires_at_ms: None,
    };
    let mut grants = StaticGrants::default();
    grants.insert("*", "default", g.clone());
    grants.insert("*", "http:fake.local", g);
    NetService::new(NetConfig {
        resolver: {
            let mut r = pii_net::StaticResolver::new();
            r.insert("fake.local", fake.addr.port(), vec![fake.addr]);
            Arc::new(r)
        },
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    })
}

fn get(fake: &Responder) -> HttpRequest {
    HttpRequest {
        method: pii_abi::HttpMethod::Get,
        url: format!("http://fake.local:{}/", fake.addr.port()),
        headers: Vec::new(),
        body: Vec::new(),
        timeout_ms: 5_000,
    }
}

/// Thousands of requests, many windows' worth of traffic in total, then a post.
#[test]
fn thousands_of_requests_then_a_post_still_succeeds() {
    let fake = responder(&format!(r#"{{"pad":"{}"}}"#, "x".repeat(2048)), POLLS + 1);
    let svc = service(&fake, WINDOW);
    let req = get(&fake);
    let ctx = net_ctx();
    for i in 0..=POLLS {
        if let Err(e) = svc.request(&ctx, &req) {
            panic!("request {i} refused: {e}");
        }
    }
}

/// Two connections on one grant, each under the window but together over it:
/// a shared counter would refuse the second.
#[test]
fn a_concurrent_connection_cannot_spend_this_ones_window() {
    let fake = responder(&format!(r#"{{"pad":"{}"}}"#, "x".repeat(700 * 1024)), 2);
    let svc = service(&fake, WINDOW);
    let ctx = net_ctx();
    let target = Target {
        host: String::from("fake.local"),
        port: fake.addr.port(),
    };
    let mut a = svc
        .connect(&ctx, &target, "http:fake.local", 5_000, Scheme::Http)
        .unwrap();
    let mut b = svc
        .connect(&ctx, &target, "http:fake.local", 5_000, Scheme::Http)
        .unwrap();
    let wire = b"GET / HTTP/1.1\r\nHost: fake.local\r\nConnection: close\r\n\r\n";
    a.write_all(wire).unwrap();
    b.write_all(wire).unwrap();
    for (name, c) in [("first", &mut a), ("second", &mut b)] {
        let mut got = 0usize;
        let mut buf = [0u8; 8192];
        loop {
            match c.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) => panic!("{name} connection refused after {got} bytes: {e}"),
            }
        }
        assert!(got > 700 * 1024, "{name} read only {got}");
    }
}

#[test]
fn a_response_larger_than_the_window_is_still_refused() {
    let fake = responder(&format!(r#"{{"pad":"{}"}}"#, "x".repeat(4096)), 1);
    let svc = service(&fake, 1024);
    let err = svc.request(&net_ctx(), &get(&fake)).unwrap_err();
    assert!(
        err.to_string().contains("byte budget"),
        "want byte-budget denial, got {err}"
    );
}

/// A responder that drains each connection to EOF, so a large client write never
/// blocks on a full socket buffer.
fn sink(connections: usize) -> Responder {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = std::io::Read::read(&mut s, &mut buf) {
                    if n == 0 {
                        break;
                    }
                }
            });
        }
    });
    Responder { addr }
}

/// Two connections on one grant, each writing under the window but together over
/// it: a counter shared across the grant would refuse the second.
#[test]
fn a_write_window_is_spent_per_connection() {
    let fake = sink(2);
    let svc = service(&fake, WINDOW);
    let ctx = net_ctx();
    let target = Target {
        host: String::from("fake.local"),
        port: fake.addr.port(),
    };
    let payload = vec![b'x'; 700 * 1024];
    let mut a = svc
        .connect(&ctx, &target, "http:fake.local", 5_000, Scheme::Http)
        .unwrap();
    let mut b = svc
        .connect(&ctx, &target, "http:fake.local", 5_000, Scheme::Http)
        .unwrap();
    for (name, c) in [("first", &mut a), ("second", &mut b)] {
        if let Err(e) = c.write_all(&payload) {
            panic!("{name} connection refused: {e}");
        }
    }
}
