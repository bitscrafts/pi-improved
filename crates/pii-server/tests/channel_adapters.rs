//! Spec 112: the webhook channel end to end through a real daemon (loopback
//! listener -> run -> reply POST to a sink), the bind policy, and `Degraded`
//! health for a misconfigured channel. Loopback only; no external network.

// reason: the printed requests are the spec 112 exit-criteria evidence
// (`cargo test -- --nocapture`).
#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pii_core::{Budget, CapKind, CapSet, Ctx, HostScope};
use pii_net::testing::FakeHttpServer;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const GRAPH: &str = r#"name = "echo"
entry = "say"
caps = []

[[node]]
id = "say"
kind = "transform"
value = "pong"
"#;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json")
}

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

fn daemon_in(dir: &Path, config_text: &str) -> Arc<Daemon> {
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    std::fs::write(dir.join("graphs/echo.toml"), GRAPH).unwrap();
    let engine = Engine::new(EngineConfig {
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        graph_dir: dir.join("graphs"),
    })
    .expect("engine");
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, config_text).unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock)).expect("daemon")
}

async fn status(engine: &Arc<Engine>) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-112".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let v = client
        .call(Method::SysStatus, serde_json::json!({}))
        .await
        .expect("sys.status");
    client.shutdown();
    v
}

/// One HTTP POST to the daemon's listener, through a loopback-granted
/// `NetService` client (sockets only via `NetService`); returns the status.
fn http(
    client: &pii_net::NetService,
    addr: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> u16 {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    caps.net_hosts = HostScope::new(vec![String::from("http://127.0.0.1")]);
    let ctx = Ctx::root(caps, Budget::unlimited());
    let req = pii_abi::HttpRequest {
        method: pii_abi::HttpMethod::Post,
        url: format!("http://{addr}{path}"),
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        body: body.as_bytes().to_vec(),
        timeout_ms: 5_000,
    };
    pii_core::HttpTransport::request(client, &ctx, &req)
        .expect("request")
        .status
}

/// A client `NetService` granted loopback access.
fn dummy() -> pii_net::NetService {
    let mut grants = pii_net::grant::StaticGrants::default();
    grants.insert("*", "default", pii_net::grant::NetGrant::open_local());
    grants.insert(
        "*",
        "http:127.0.0.1",
        pii_net::grant::NetGrant::open_local(),
    );
    pii_net::NetService::new(pii_net::NetConfig {
        resolver: Arc::new(pii_net::SystemResolver),
        grants: Arc::new(grants),
        clock: Arc::new(pii_core::SystemClock),
    })
}

fn webhook_addr(st: &serde_json::Value) -> String {
    st["listeners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["addr"].as_str().unwrap().to_owned())
        .find(|a| !a.is_empty() && !a.contains('/'))
        .expect("a webhook listener in sys.status")
}

#[tokio::test(flavor = "multi_thread")]
async fn webhook_post_makes_a_run_and_the_reply_is_posted_to_the_sink() {
    let sink = FakeHttpServer::start(
        r#"{"routes":[{"method":"POST","path_prefix":"/reply","status":200,"body":{"ok":true}}]}"#,
    )
    .unwrap();
    let dir = scratch("wh-e2e-");
    let cfg = format!(
        "[net]\ngrants = [\"127.0.0.1:*\"]\nallow_private = true\n\n\
         [[channel]]\nname = \"hook\"\nkind = \"webhook\"\ngraph = \"echo\"\n\
         poll_interval_ms = 20\nbind = \"127.0.0.1:0\"\nurl = \"http://127.0.0.1:{}/reply\"\n",
        sink.addr.port()
    );
    let d = daemon_in(dir.path(), &cfg);
    let _h = d.spawn_background();
    let st = status(&d.engine).await;
    let addr = webhook_addr(&st);
    assert!(
        addr.starts_with("127.0.0.1:"),
        "loopback by default: {addr}"
    );
    let code = http(
        &dummy(),
        &addr,
        "/channel/hook",
        &[("Content-Type", "application/json")],
        r#"{"text":"ping","chat":"room1","user":"ann"}"#,
    );
    assert_eq!(code, 202);
    let mut got = Vec::new();
    for _ in 0..100 {
        got = sink.requests();
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(got.len(), 1, "exactly one reply POST reached the sink");
    println!(
        "INBOUND  POST /channel/hook {{\"text\":\"ping\",\"chat\":\"room1\",\"user\":\"ann\"}} -> 202"
    );
    println!(
        "OUTBOUND {} {} {}",
        got[0].method,
        got[0].path,
        String::from_utf8_lossy(&got[0].body)
    );
    assert_eq!(got[0].method, "POST");
    assert_eq!(got[0].path, "/reply");
    let body: serde_json::Value = serde_json::from_slice(&got[0].body).unwrap();
    assert_eq!(body["to"], "room1");
    assert!(body["text"].as_str().unwrap().contains("pong"));
}

#[tokio::test(flavor = "multi_thread")]
async fn non_loopback_bind_refuses_without_the_secret_and_accepts_with_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("wh-wide-");
    let secret = dir.path().join("hook.secret");
    std::fs::write(&secret, "s3cret-value\n").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    let cfg = format!(
        "[[channel]]\nname = \"hook\"\nkind = \"webhook\"\ngraph = \"echo\"\n\
         bind = \"0.0.0.0:0\"\nsecret_file = \"{}\"\n",
        secret.display()
    );
    let d = daemon_in(dir.path(), &cfg);
    let st = status(&d.engine).await;
    let addr = webhook_addr(&st).replace("0.0.0.0", "127.0.0.1");
    let c = dummy();
    let code = http(&c, &addr, "/channel/hook", &[], r#"{"text":"x"}"#);
    println!("wide bind, no secret   -> {code}");
    assert_eq!(code, 401);
    let code = http(
        &c,
        &addr,
        "/channel/hook",
        &[("X-Pii-Secret", "nope")],
        r#"{"text":"x"}"#,
    );
    assert_eq!(code, 401);
    let code = http(
        &c,
        &addr,
        "/channel/hook",
        &[("X-Pii-Secret", "s3cret-value")],
        r#"{"text":"x"}"#,
    );
    println!("wide bind, with secret -> {code}");
    assert_eq!(code, 202);
    let v = status(&d.engine).await;
    assert!(
        !v.to_string().contains("s3cret-value"),
        "secret leaked into health"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn misconfigured_channels_are_degraded_in_health_not_silently_ok() {
    let dir = scratch("wh-degraded-");
    let cfg = "[[channel]]\nname = \"tg\"\nkind = \"telegram\"\ngraph = \"echo\"\n\
               token_env = \"PII_TEST_NO_SUCH_TOKEN_112\"\n\n\
               [[channel]]\nname = \"wide\"\nkind = \"webhook\"\ngraph = \"echo\"\n\
               bind = \"0.0.0.0:0\"\n";
    let d = daemon_in(dir.path(), cfg);
    let st = status(&d.engine).await;
    let subs = st["subsystems"].as_array().unwrap();
    for name in ["channel.tg", "channel.wide"] {
        let s = subs
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from {subs:?}"));
        println!("health {name}: {} {}", s["state"], s["detail"]);
        assert_eq!(s["state"], "degraded", "{s}");
        assert!(s["detail"].as_str().unwrap().len() > 3);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reply_to_a_host_outside_the_allowlist_is_refused_not_attempted() {
    let sink = FakeHttpServer::start(
        r#"{"routes":[{"method":"POST","path_prefix":"/reply","status":200,"body":{"ok":true}}]}"#,
    )
    .unwrap();
    let dir = scratch("wh-egress-");
    // No [net] grants: the sink's host is not on the allowlist.
    let cfg = format!(
        "[[channel]]\nname = \"hook\"\nkind = \"webhook\"\ngraph = \"echo\"\n\
         poll_interval_ms = 20\nbind = \"127.0.0.1:0\"\nurl = \"http://127.0.0.1:{}/reply\"\n",
        sink.addr.port()
    );
    let d = daemon_in(dir.path(), &cfg);
    let _h = d.spawn_background();
    let addr = webhook_addr(&status(&d.engine).await);
    assert_eq!(
        http(&dummy(), &addr, "/channel/hook", &[], r#"{"text":"ping"}"#),
        202
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        sink.requests().is_empty(),
        "the denied destination must never be dialled"
    );
}
