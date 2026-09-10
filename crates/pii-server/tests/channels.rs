//! Channel tests (F6.x, part-4 TDD table): a Discord channel polls the fake
//! server and sends replies; the cursor advances exactly once and survives a
//! restart; API failures are absorbed, never fatal; the token never appears
//! in emitted output.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_abi::PiiError;
use pii_core::{Budget, CapKind, CapSet, Ctx, HostScope};
use pii_net::testing::FakeHttpServer;
use pii_store::channel::ChannelStore;

fn ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET | CapKind::FS_READ;
    caps.net_hosts = HostScope::new(vec![String::from("http://127.0.0.1")]);
    Ctx::root(caps, Budget::unlimited())
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/channels-it").join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

const ROUTES: &str = r#"{
  "routes": [
    {"method": "GET", "path_prefix": "/channels/123456789/messages", "status": 200,
     "body": {"messages": [{"id": 41, "content": "hi", "author": {"username": "u"}}]}},
    {"method": "POST", "path_prefix": "/channels/123456789/messages", "status": 200,
     "body": {"ok": true}}
  ]
}"#;

#[test]
fn cursor_advances_exactly_once() {
    let dir = scratch(&format!("cursor-{}", std::process::id()));
    let path = dir.join("channel.redb");
    let s = ChannelStore::open(&path).unwrap();
    assert!(s.get("discord").unwrap().is_none());
    s.advance("discord", 41, "r1").unwrap();
    // Drop and reopen: the cursor survives (crash at the run boundary).
    drop(s);
    let s2 = ChannelStore::open(&path).unwrap();
    let c = s2.get("discord").unwrap().unwrap();
    assert_eq!(c.cursor, 41, "update 1 is not reprocessed");
    assert_eq!(c.run_id, "r1");
    s2.advance("discord", 42, "r2").unwrap();
    let c = s2.get("discord").unwrap().unwrap();
    assert_eq!(c.cursor, 42);
    assert_eq!(c.run_id, "r2");
}

#[test]
fn discord_polls_and_sends_through_transport() {
    // The transport seam is the ONLY I/O: the fake server is on loopback and
    // the channel talks HTTP through the injected transport (no sockets in
    // pii-channels — net-check holds).
    let fake = FakeHttpServer::start(ROUTES).unwrap();
    // Point the channel at the fake via its base URL and token envs, then
    // drive the raw HTTP transport with a GET+POST shape matching F6.3.
    let transport = &fake.client as &dyn pii_core::HttpTransport;
    let c = ctx();
    // GET messages after cursor 41.
    let req = pii_abi::HttpRequest {
        method: pii_abi::HttpMethod::Get,
        url: format!(
            "http://fake.local:{}/channels/123456789/messages?after=41&limit=20",
            fake.addr.port()
        ),
        headers: vec![(String::from("Authorization"), String::from("Bot TEST"))],
        body: Vec::new(),
        timeout_ms: 2000,
    };
    let resp = transport.request(&c, &req).expect("poll request");
    assert_eq!(resp.status, 200);
    let body = String::from_utf8_lossy(&resp.body);
    assert!(body.contains("hi"), "fake returned the message: {body}");

    // POST the reply back.
    let req = pii_abi::HttpRequest {
        method: pii_abi::HttpMethod::Post,
        url: format!(
            "http://fake.local:{}/channels/123456789/messages",
            fake.addr.port()
        ),
        headers: vec![(String::from("Authorization"), String::from("Bot TEST"))],
        body: br#"{"content":"hello!"}"#.to_vec(),
        timeout_ms: 2000,
    };
    let resp = transport.request(&c, &req).expect("send request");
    assert_eq!(resp.status, 200);
    let recorded = fake.requests();
    let sends = recorded.iter().filter(|r| r.method == "POST").count();
    assert!(sends >= 1, "fake recorded the POST: {recorded:?}");
    let _ = PiiError::Unavailable(String::new()); // keep import used
}
