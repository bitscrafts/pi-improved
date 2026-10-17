//! Spec 128: the `post` node end to end through a real daemon. A message arrives on a channel
//! (the `cli` loopback stands in for Discord: same `Channel` trait, same hub, no token), the
//! committed `graphs/discord-triage.toml` classifies it by rules and posts the class back into
//! the same thread; `graphs/discord-digest.toml` posts a digest. The retry, timeout, capability
//! and counter claims are measured here. The one test that talks to real Discord is `#[ignore]`d.

#![allow(
    clippy::print_stdout,
    clippy::disallowed_macros,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use pii_channels::{Channel, Incoming};
use pii_core::{Budget, CapSet, Ctx};
use pii_observe::{Recorder, names};
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;
use pii_server::daemon::Daemon;
use pii_server::daemon::config::Config;
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const TRIAGE: &str = include_str!("../../../graphs/discord-triage.toml");
/// The shipped digest starts at a `timer` (spec 176), which suspends a direct run; this test
/// measures the digest's body, so it enters at the node the timer feeds.
const DIGEST_SHIPPED: &str = include_str!("../../../graphs/discord-digest.toml");
const REMEMBER: &str = include_str!("../../../examples/memory-remember.toml");

/// A post node with its own caps removed, under a graph that grants `post`.
const NODE_DENIED: &str = r#"name = "post-denied"
entry = "p"
caps = ["post"]

[[node]]
id = "p"
kind = "post"
channel = "discord"
to = "t1"
text = "hello"
caps = []
"#;

const NO_GRANT: &str = r#"name = "post-nogrant"
entry = "p"

[[node]]
id = "p"
kind = "post"
channel = "discord"
to = "t1"
text = "hello"
"#;

/// The retry graph: the first send fails, the engine retries the node, the key keeps it to one post.
const RETRY: &str = r#"name = "post-retry"
entry = "p"
caps = ["post"]

[[node]]
id = "p"
kind = "post"
channel = "flaky"
to = "t9"
text = "once"
key = "retry-demo"
timeout_ms = 150
retry = { max = 2 }
"#;

fn scratch(p: &str) -> tempfile::TempDir {
    pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), p).expect("tempdir")
}

/// A channel whose behaviour per send is scripted: fail, stall, or deliver.
struct Flaky {
    delivered: Mutex<Vec<(String, String)>>,
    fail_first: AtomicU64,
    stall_first_ms: AtomicU64,
}

impl Channel for Flaky {
    fn name(&self) -> &'static str {
        "flaky"
    }
    fn kind(&self) -> &'static str {
        "mock"
    }
    fn poll(&self, _: &Ctx, _: Option<u64>) -> Result<Vec<Incoming>, pii_abi::PiiError> {
        Ok(Vec::new())
    }
    fn send(&self, _: &Ctx, to: &str, text: &str) -> Result<(), pii_abi::PiiError> {
        let stall = self.stall_first_ms.swap(0, Ordering::SeqCst);
        if stall > 0 {
            std::thread::sleep(Duration::from_millis(stall));
        }
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(pii_abi::PiiError::Unavailable(String::from(
                "discord: send status 503: upstream hiccup",
            )));
        }
        self.delivered.lock().push((to.to_owned(), text.to_owned()));
        Ok(())
    }
}

fn daemon_in(dir: &Path, config_text: &str) -> (Arc<Daemon>, Arc<Recorder>) {
    let hello = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase2/hello.json");
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::create_dir_all(dir.join("graphs")).unwrap();
    let digest = DIGEST_SHIPPED
        .replacen("entry = \"tick\"", "entry = \"devices\"", 1)
        .replacen(
            "[[node]]\nid = \"tick\"\nkind = \"timer\"\ncron = \"0 9 * * *\"\n\n",
            "",
            1,
        )
        .replacen(
            "[[edge]]\nfrom = \"tick\"\nport = \"fired\"\nto = \"devices\"\nkind = \"data\"\n\n",
            "",
            1,
        );
    assert!(!digest.contains("kind = \"timer\""), "timer not stripped");
    for (n, g) in [
        ("discord-triage", TRIAGE),
        ("discord-digest", digest.as_str()),
        ("remember", REMEMBER),
        ("post-denied", NODE_DENIED),
        ("post-nogrant", NO_GRANT),
        ("post-retry", RETRY),
    ] {
        std::fs::write(dir.join(format!("graphs/{n}.toml")), g).unwrap();
    }
    let engine = Engine::new(EngineConfig {
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
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let cfg_path = dir.join("pii.toml");
    std::fs::write(&cfg_path, config_text).unwrap();
    let cfg = Config::load(&cfg_path).expect("config");
    let d = Daemon::build(engine, &cfg, dir, cfg_path, Arc::new(pii_core::SystemClock))
        .expect("daemon");
    (d, log)
}

/// A `cli` channel named `discord`: the loopback that stands in for the Discord API.
fn loopback_cfg(dir: &Path, graph: &str) -> (String, PathBuf, PathBuf) {
    let root = dir.join("channels/discord");
    std::fs::create_dir_all(&root).unwrap();
    let (inbox, outbox) = (root.join("in.jsonl"), root.join("out.jsonl"));
    let cfg = format!(
        "[[channel]]\nname = \"discord\"\nkind = \"cli\"\ngraph = \"{graph}\"\npoll_interval_ms = 20\n\
         inbox = \"{}\"\noutbox = \"{}\"\n",
        inbox.display(),
        outbox.display()
    );
    (cfg, inbox, outbox)
}

async fn call(engine: &Arc<Engine>, m: Method, params: serde_json::Value) -> serde_json::Value {
    let stream = pii_server::engine::loopback::spawn_loopback(Arc::clone(engine));
    let (r, w) = tokio::io::split(stream);
    let info = PeerInfo {
        name: "spec-128".into(),
        version: "0".into(),
        build: "test".into(),
    };
    let (client, _) = pii_proto::client::Client::connect(r, w, info)
        .await
        .expect("connect");
    let out = client.call(m, params).await;
    client.shutdown();
    out.unwrap_or_else(|e| serde_json::json!({"error": format!("{e:?}")}))
}

async fn run_graph(
    e: &Arc<Engine>,
    graph: &str,
    input: serde_json::Value,
) -> (String, serde_json::Value, serde_json::Value) {
    let r = call(
        e,
        Method::RunStart,
        serde_json::json!({"graph": graph, "input": input}),
    )
    .await;
    let Some(rid) = r["run_id"].as_str().map(str::to_owned) else {
        return (String::new(), r, serde_json::Value::Null);
    };
    for _ in 0..300 {
        let h = call(e, Method::RunGet, serde_json::json!({"run_id": rid})).await;
        if matches!(h["status"].as_str(), Some("succeeded" | "failed")) {
            let t = call(e, Method::RunTrace, serde_json::json!({"run_id": rid})).await;
            return (rid, h, t);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("run {rid} never terminated");
}

fn lines(p: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_is_classified_by_rules_and_posted_back_into_its_thread() {
    let dir = scratch("post-triage-");
    let (cfg, inbox, outbox) = loopback_cfg(dir.path(), "discord-triage");
    let (d, log) = daemon_in(dir.path(), &cfg);
    std::fs::write(
        &inbox,
        concat!(
            r#"{"chat":"thread-111","user":"m","text":"is the build green?","cursor":1}"#,
            "\n",
            r#"{"chat":"thread-222","user":"m","text":"[ALERT] disk is full","cursor":2}"#,
            "\n",
            r#"{"chat":"thread-333","user":"m","text":"good morning","cursor":3}"#,
            "\n"
        ),
    )
    .unwrap();
    let _h = d.spawn_background();
    for _ in 0..300 {
        if lines(&outbox).len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // give a stray automatic reply time to show up: there must be none.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let out = lines(&outbox);
    println!("CHANNEL VIEW (what the thread received): {out:?}");
    assert_eq!(
        out.len(),
        3,
        "one post per message, no automatic echo: {out:?}"
    );
    let by_to = |to: &str| {
        out.iter()
            .find(|l| l["to"] == to)
            .unwrap_or_else(|| panic!("nothing posted to {to}: {out:?}"))["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(by_to("thread-111"), "[question] is the build green?");
    assert_eq!(by_to("thread-222"), "[alert] [ALERT] disk is full");
    assert_eq!(by_to("thread-333"), "[chat] good morning");

    let runs = call(&d.engine, Method::RunList, serde_json::json!({})).await;
    let rid = runs["runs"][0]["run_id"]
        .as_str()
        .or_else(|| runs[0]["run_id"].as_str())
        .unwrap()
        .to_owned();
    let trace = call(
        &d.engine,
        Method::RunTrace,
        serde_json::json!({"run_id": rid}),
    )
    .await;
    let text = trace.to_string();
    for part in text
        .split("},{")
        .filter(|l| l.contains("node_finished") || l.contains("NodeFinished"))
    {
        println!("JOURNAL {rid}: {part}");
    }
    assert!(
        text.contains("\"class\"") && text.contains("\"bytes\""),
        "{text}"
    );
    assert!(
        text.contains("\"to\":\"thread-"),
        "the journal carries the destination: {text}"
    );

    let ev = log.events_named(names::CHANNEL_POSTED);
    for e in &ev {
        println!("EVENT channel.posted {:?}", e.fields);
    }
    assert_eq!(ev.len(), 3);
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    println!(
        "COUNTS: channel_posts={} channel_post_refusals={}",
        st["counts"]["channel_posts"], st["counts"]["channel_post_refusals"]
    );
    assert_eq!(st["counts"]["channel_posts"], 3);
    assert_eq!(st["counts"]["channel_post_refusals"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_digest_graph_posts_what_the_memory_graph_holds() {
    let dir = scratch("post-digest-");
    let (cfg, _inbox, outbox) = loopback_cfg(dir.path(), "discord-triage");
    let (d, log) = daemon_in(dir.path(), &cfg);
    let (_, w, _) = run_graph(&d.engine, "remember", serde_json::json!({"note": "seed"})).await;
    assert_eq!(w["status"], "succeeded", "{w}");
    let (rid, run, trace) = run_graph(
        &d.engine,
        "discord-digest",
        serde_json::json!({"to": "thread-digest"}),
    )
    .await;
    println!("DIGEST run {rid}: {run}");
    for part in trace
        .to_string()
        .split("},{")
        .filter(|l| l.contains("thread-digest") || l.contains("count"))
    {
        println!("JOURNAL: {part}");
    }
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");
    let out = lines(&outbox);
    println!("DIGEST TEXT POSTED: {out:?}");
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0]["to"], "thread-digest");
    assert_eq!(
        out[0]["text"],
        "pii digest: 1 device(s) known, 0 lesson(s) remembered. (inputs: recall by_kind device, lesson)"
    );
    assert_eq!(log.events_named(names::CHANNEL_POSTED).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_post_without_the_capability_is_refused_by_name_and_counted() {
    let dir = scratch("post-denied-");
    let (cfg, _i, outbox) = loopback_cfg(dir.path(), "discord-triage");
    let (d, log) = daemon_in(dir.path(), &cfg);
    let (rid, run, trace) = run_graph(&d.engine, "post-denied", serde_json::json!({})).await;
    println!("NODE WITHOUT CAP run {rid}: {run}\n  journal: {trace}");
    assert_eq!(run["status"], "failed");
    assert!(
        trace
            .to_string()
            .contains("capability denied: post: channel discord not granted"),
        "{trace}"
    );
    let (_, refused, trace) = run_graph(&d.engine, "post-nogrant", serde_json::json!({})).await;
    println!("GRAPH WITHOUT GRANT: {refused}\n  journal: {trace}");
    assert_eq!(refused["status"], "failed", "{refused}");
    assert!(trace.to_string().contains("post"), "{trace}");
    assert!(lines(&outbox).is_empty(), "nothing reached the channel");
    assert_eq!(log.events_named(names::CHANNEL_POST_REFUSED).len(), 1);
    let st = call(&d.engine, Method::SysStatus, serde_json::json!({})).await;
    println!("COUNTS: {}", st["counts"]);
    assert_eq!(st["counts"]["channel_post_refusals"], 1);
    assert_eq!(st["counts"]["channel_posts"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_engine_retry_of_a_failed_post_delivers_exactly_once() {
    let dir = scratch("post-retry-");
    let (d, log) = daemon_in(dir.path(), "# defaults\n");
    let flaky = Arc::new(Flaky {
        delivered: Mutex::new(Vec::new()),
        fail_first: AtomicU64::new(1),
        stall_first_ms: AtomicU64::new(0),
    });
    d.engine.channel_hub.insert(
        Arc::clone(&flaky) as Arc<dyn Channel>,
        Ctx::root(CapSet::empty(), Budget::unlimited()),
    );
    let (rid, run, trace) = run_graph(&d.engine, "post-retry", serde_json::json!({})).await;
    println!("RETRY run {rid}: {run}");
    for part in trace
        .to_string()
        .split("},{")
        .filter(|l| l.contains("503") || l.contains("duplicate"))
    {
        println!("  journal: {part}");
    }
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");
    assert!(
        trace.to_string().contains("503"),
        "the failed attempt is journaled: {trace}"
    );
    assert_eq!(
        *flaky.delivered.lock(),
        vec![("t9".to_owned(), "once".to_owned())]
    );
    assert_eq!(log.events_named(names::CHANNEL_POST_REFUSED).len(), 1);
    assert_eq!(log.events_named(names::CHANNEL_POSTED).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_send_times_out_and_the_retry_does_not_double_post() {
    let dir = scratch("post-stall-");
    let (d, _log) = daemon_in(dir.path(), "# defaults\n");
    let flaky = Arc::new(Flaky {
        delivered: Mutex::new(Vec::new()),
        fail_first: AtomicU64::new(0),
        stall_first_ms: AtomicU64::new(700),
    });
    d.engine.channel_hub.insert(
        Arc::clone(&flaky) as Arc<dyn Channel>,
        Ctx::root(CapSet::empty(), Budget::unlimited()),
    );
    let t0 = std::time::Instant::now();
    let (rid, run, trace) = run_graph(&d.engine, "post-retry", serde_json::json!({})).await;
    let elapsed = t0.elapsed().as_millis();
    println!(
        "STALL run {rid}: {run} in {elapsed} ms (the channel stalls 700 ms, timeout_ms = 150)"
    );
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");
    assert!(
        trace.to_string().contains("deadline exceeded after 150ms"),
        "{trace}"
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(
        flaky.delivered.lock().len(),
        1,
        "the stalled send landed once; the retry saw the key"
    );
}

/// Live: needs `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID` and `PII_DISCORD_TEST_THREAD` (a thread or
/// text channel the bot may write to). Run with `cargo test -p pii-server --test post_e2e -- --ignored`.
/// It posts one real message. Never part of the gate.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "posts to real Discord: needs PII_DISCORD_TOKEN, PII_DISCORD_CHANNEL_ID, PII_DISCORD_TEST_THREAD"]
async fn live_discord_post() {
    let thread = pii_core::env_var("PII_DISCORD_TEST_THREAD").expect("PII_DISCORD_TEST_THREAD");
    let dir = scratch("post-live-");
    let cfg = "[[channel]]\nname = \"discord\"\nkind = \"discord\"\ngraph = \"discord-triage\"\n";
    let (d, _log) = daemon_in(dir.path(), cfg);
    std::fs::write(
        dir.path().join("graphs/live.toml"),
        "name = \"live\"\nentry = \"p\"\ncaps = [\"post\"]\n[[node]]\nid = \"p\"\nkind = \"post\"\nchannel = \"discord\"\nto = \"$.input.to\"\ntext = \"pii spec 128 live post test\"\n",
    )
    .unwrap();
    let (rid, run, trace) = run_graph(&d.engine, "live", serde_json::json!({"to": thread})).await;
    println!("LIVE run {rid}: {run}\n{trace}");
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");
}

/// Live (spec 132): needs `PII_DISCORD_TOKEN`, `PII_DISCORD_CHANNEL_ID` and `PII_DISCORD_FORUM` (a forum
/// the bot may post into; its tag comes from `PII_DISCORD_FORUM_TAG`, default `chat`). Creates one real
/// thread. Never part of the gate.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "posts to real Discord: needs PII_DISCORD_TOKEN, PII_DISCORD_CHANNEL_ID, PII_DISCORD_FORUM"]
async fn live_discord_forum_post() {
    let forum = pii_core::env_var("PII_DISCORD_FORUM").expect("PII_DISCORD_FORUM");
    let dir = scratch("post-forum-live-");
    let cfg = "[[channel]]\nname = \"discord\"\nkind = \"discord\"\ngraph = \"discord-triage\"\n";
    let (d, _log) = daemon_in(dir.path(), cfg);
    std::fs::write(
        dir.path().join("graphs/live.toml"),
        "name = \"live\"\nentry = \"p\"\ncaps = [\"post\"]\n[[node]]\nid = \"p\"\nkind = \"post\"\nchannel = \"discord\"\nto = \"$.input.to\"\ntext = \"pii spec 132 live forum post\"\n",
    )
    .unwrap();
    let (rid, run, trace) = run_graph(&d.engine, "live", serde_json::json!({"to": forum})).await;
    println!("LIVE run {rid}: {run}\n{trace}");
    assert_eq!(run["status"], "succeeded", "{run}\n{trace}");
}

/// Spec 132, on the production path: the egress a channel declares reaches the daemon's
/// `NetService` grants for that channel's principal — and only that host, only that principal.
/// (The channel's own `Ctx` caps were never what the network layer checked; its `GrantStore` is.)
#[test]
fn a_channel_may_dial_the_host_it_declares_and_no_other() {
    use pii_abi::{HttpMethod, HttpRequest, PiiError};
    use pii_core::{
        Budget, BudgetShare, CapKind, CapSet, Ctx, HostScope, HttpTransport, Principal,
    };

    let dir = scratch("channel-egress-");
    // No `[net] grants` at all: the only grant a channel can have is the one it declares.
    let cfg = "[[channel]]\nname = \"tg\"\nkind = \"telegram\"\ngraph = \"discord-triage\"\n\
               base_url = \"http://203.0.113.5:8080\"\n";
    let (d, _log) = daemon_in(dir.path(), cfg);
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    caps.net_hosts = HostScope::new(vec![String::new()]);
    let as_principal = |p: Principal| {
        Ctx::root(caps.clone(), Budget::unlimited()).child(p, &caps, &BudgetShare::Shared)
    };
    let get = |url: &str| HttpRequest {
        method: HttpMethod::Get,
        url: url.to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
        timeout_ms: 300,
    };
    let tg = as_principal(Principal::Channel(String::from("tg")));
    let denied =
        |r: Result<_, PiiError>| matches!(r, Err(PiiError::Denied(m)) if m.contains("grant"));
    assert!(
        !denied(d.ctx.net.request(&tg, &get("http://203.0.113.5:8080/x"))),
        "the declared host is granted"
    );
    assert!(
        denied(d.ctx.net.request(&tg, &get("http://203.0.113.6:8080/x"))),
        "another host is not"
    );
    let other = as_principal(Principal::Channel(String::from("other")));
    assert!(
        denied(d.ctx.net.request(&other, &get("http://203.0.113.5:8080/x"))),
        "another principal does not inherit it"
    );
}
