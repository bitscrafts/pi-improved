//! The discord channel over the real net service (spec 180): a poller that has
//! made thousands of requests, whose traffic is many times the byte window, can
//! still post; a response larger than the window is still refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    // reason: the test serves its own loopback responder; grants do not apply
    // to the test's own listener.
    clippy::disallowed_methods
)]

use std::sync::Arc;

use pii_channels::Channel as _;
use pii_channels::discord::DiscordChannel;
use pii_core::{Budget, CapKind, CapSet, Ctx};
use pii_net::grant::{HostScope, NetGrant, StaticGrants};
use pii_net::{NetConfig, NetService};

const WINDOW: u64 = 64 * 1024;
const POLLS: usize = 3_000;

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}

/// Answers by request line: a message list for the poll, a text-channel shape
/// for the probe, `{}` for the post. `poll_body` is the poll's response body.
fn responder(poll_body: String, connections: usize) -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut seen = Vec::new();
            let mut buf = [0u8; 2048];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match std::io::Read::read(&mut s, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            let line = String::from_utf8_lossy(&seen);
            let first = line.lines().next().unwrap_or_default();
            let body = if first.contains("/messages") && first.starts_with("GET") {
                poll_body.clone()
            } else if first.starts_with("GET") {
                String::from(r#"{"type":0}"#)
            } else {
                String::from("{}")
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut s, resp.as_bytes());
        }
    });
    addr
}

fn channel(addr: std::net::SocketAddr, window: u64) -> DiscordChannel {
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
    let mut resolver = pii_net::StaticResolver::new();
    resolver.insert("fake.local", addr.port(), vec![addr]);
    let svc = NetService::new(NetConfig {
        resolver: Arc::new(resolver),
        grants: Arc::new(grants),
        clock: Arc::new(pii_net::FixedClock::new()),
    });
    let base = format!("http://fake.local:{}", addr.port());
    DiscordChannel::from_lookup("discord", Arc::new(svc), |k| match k {
        "PII_DISCORD_TOKEN" => Some(String::from("t")),
        "PII_DISCORD_CHANNEL_ID" => Some(String::from("42")),
        "PII_DISCORD_API_BASE" => Some(base.clone()),
        _ => None,
    })
    .unwrap()
}

#[test]
fn a_channel_that_polled_thousands_of_times_can_still_post() {
    let addr = responder(String::from("[]"), POLLS + 2);
    let ch = channel(addr, WINDOW);
    let ctx = ctx();
    for i in 0..POLLS {
        if let Err(e) = ch.poll(&ctx, None) {
            panic!("poll {i} of {POLLS} refused: {e}");
        }
    }
    ch.send(&ctx, "42", "the digest")
        .unwrap_or_else(|e| panic!("post after {POLLS} polls refused: {e}"));
}

#[test]
fn a_poll_response_larger_than_the_window_is_still_refused() {
    let big = format!(
        r#"[{{"id":"1","content":"{}","author":{{"username":"u"}}}}]"#,
        "x".repeat(4096)
    );
    let addr = responder(big, 1);
    let ch = channel(addr, 1024);
    let err = ch.poll(&ctx(), None).unwrap_err();
    assert!(
        err.to_string().contains("byte budget"),
        "want byte-budget denial, got {err}"
    );
}
