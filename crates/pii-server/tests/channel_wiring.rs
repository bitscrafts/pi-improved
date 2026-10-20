//! Spec 157: the deployed daemon posts to the operator's channel. With the Discord credentials
//! absent the daemon still builds and reports no channel (the control); with them present it
//! starts the channel and records one verification post that names the deployed revision.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;

use pii_net::testing::FakeHttpServer;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

/// A made-up token: the leak check below looks for exactly this value.
const TOKEN: &str = "spec157-not-a-real-token";
const CHANNEL: &str = "424242";

const ROUTES: &str = r#"{
  "routes": [
    {"method": "GET", "path_prefix": "/channels/424242", "status": 200, "body": {"type": 0}},
    {"method": "POST", "path_prefix": "/channels/424242/messages", "status": 200, "body": {"id": "1"}}
  ]
}"#;

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

fn build(
    dir: &Path,
    port: u16,
    env: &dyn Fn(&str) -> Option<String>,
) -> (Arc<Engine>, Arc<Daemon>) {
    let hello = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.join("ws"),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello.display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.join("graphs"),
    })
    .expect("engine");
    let cfg_path = dir.join("pii.toml");
    std::fs::write(
        &cfg_path,
        format!(
            "[net]\nallow_private = true\n\n\
             [[channel]]\nname = \"discord\"\nkind = \"discord\"\ngraph = \"g\"\n\
             hosts = [\"127.0.0.1:{port}\"]\n"
        ),
    )
    .unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    let d = Daemon::build_with_env(
        Arc::clone(&engine),
        &cfg,
        dir,
        cfg_path,
        Arc::new(pii_core::SystemClock),
        env,
    )
    .expect("a missing credential must not fail the build");
    (engine, d)
}

async fn status(engine: &Arc<Engine>) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-157".into(),
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

#[tokio::test(flavor = "multi_thread")]
async fn no_credentials_the_daemon_starts_with_no_channel_and_posts_nothing() {
    let fake = FakeHttpServer::start(ROUTES).unwrap();
    let tmp = scratch("wiring-none");
    let (engine, d) = build(tmp.path(), fake.addr.port(), &|_| None);
    let st = status(&engine).await;
    assert_eq!(st["channels"], serde_json::json!([]), "{st}");
    let posted = d.post_verification(&|_| None);
    assert!(posted.is_none(), "no channel, no post attempted");
    assert!(fake.requests().is_empty(), "nothing reached the network");
    let st = status(&engine).await;
    assert_eq!(st["counts"]["channel_posts"], 0);
    assert_eq!(st["counts"]["channel_post_refusals"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_present_one_post_naming_the_revision_is_recorded() {
    let fake = FakeHttpServer::start(ROUTES).unwrap();
    let base = format!("http://127.0.0.1:{}", fake.addr.port());
    let env = move |k: &str| match k {
        "PII_DISCORD_TOKEN" => Some(TOKEN.to_owned()),
        "PII_DISCORD_CHANNEL_ID" => Some(CHANNEL.to_owned()),
        "PII_DISCORD_API_BASE" => Some(base.clone()),
        _ => None,
    };
    let tmp = scratch("wiring-some");
    let (engine, d) = build(tmp.path(), fake.addr.port(), &env);
    let st = status(&engine).await;
    assert_eq!(st["channels"][0]["kind"], "discord", "{st}");

    let d2 = Arc::clone(&d);
    let outcome = tokio::task::spawn_blocking(move || d2.post_verification(&env))
        .await
        .unwrap();
    assert!(matches!(outcome, Some(Ok(_))), "{outcome:?}");

    let st = status(&engine).await;
    assert_eq!(st["counts"]["channel_posts"], 1, "{st}");
    let sent: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(sent.len(), 1, "exactly one verification post");
    let body = String::from_utf8_lossy(&sent[0].body).into_owned();
    assert!(
        body.contains(pii_server::daemon::identity::REVISION),
        "the text names the deployed revision: {body}"
    );
    assert!(!body.contains(TOKEN), "the token is never in the text");

    // The token value is nowhere in what the daemon wrote: the runtime dir and the store.
    for entry in walk(tmp.path()) {
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !bytes.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()),
            "token value found in {}",
            entry.display()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_verification_post_is_counted_not_fatal() {
    let fake = FakeHttpServer::start(r#"{"routes": []}"#).unwrap();
    let base = format!("http://127.0.0.1:{}", fake.addr.port());
    let env = move |k: &str| match k {
        "PII_DISCORD_TOKEN" => Some(TOKEN.to_owned()),
        "PII_DISCORD_CHANNEL_ID" => Some(CHANNEL.to_owned()),
        "PII_DISCORD_API_BASE" => Some(base.clone()),
        _ => None,
    };
    let tmp = scratch("wiring-refused");
    let (engine, d) = build(tmp.path(), fake.addr.port(), &env);
    let d2 = Arc::clone(&d);
    let outcome = tokio::task::spawn_blocking(move || d2.post_verification(&env))
        .await
        .unwrap();
    assert!(matches!(outcome, Some(Err(_))), "{outcome:?}");
    let st = status(&engine).await;
    assert_eq!(st["counts"]["channel_posts"], 0);
    assert_eq!(st["counts"]["channel_post_refusals"], 1);
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            for e in std::fs::read_dir(&p).unwrap().flatten() {
                stack.push(e.path());
            }
        } else {
            out.push(p);
        }
    }
    out
}
