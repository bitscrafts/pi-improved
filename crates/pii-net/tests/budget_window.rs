//! The byte budget is a window per connection, not a total for the life of the
//! process: a long-lived poller must not exhaust it, and one response larger
//! than the window must still be refused.

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
use pii_net::{NetConfig, NetService};

const WINDOW: u64 = 1024 * 1024;
const POLLS: usize = 50_000;

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

/// Cumulative traffic here is several times the window, so a process-lifetime
/// budget would refuse long before the last poll.
#[test]
fn fifty_thousand_polls_never_exhaust_the_budget() {
    let polls = POLLS;
    let fake = responder(r#"{"ok":true}"#, polls);
    let svc = service(&fake, WINDOW);
    let req = get(&fake);
    let ctx = net_ctx();
    for i in 0..polls {
        if let Err(e) = svc.request(&ctx, &req) {
            panic!("poll {i} of {polls} refused: {e}");
        }
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
