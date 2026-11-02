//! Spec 194: a rate-limited poll waits for what the provider said. The loop is the daemon's own
//! `spawn_all`; the channel is a fake that fails with the real Discord 429 text.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pii_abi::PiiError;
use pii_channels::{Channel, Incoming};
use pii_server::daemon::status::{ChannelRuntime, DaemonContext};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const REAL_429: &str = r#"discord: poll status 429: {"message":"You are being rate limited.","retry_after":1.0,"global":false}"#;

/// Fails the first poll with the 429 text, then succeeds; counts every call.
struct Limited {
    calls: Arc<AtomicUsize>,
}

impl Channel for Limited {
    fn name(&self) -> &'static str {
        "limited"
    }
    fn kind(&self) -> &'static str {
        "limited-test"
    }
    fn poll(&self, _ctx: &pii_core::Ctx, _cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(PiiError::Unavailable(String::from(REAL_429)))
        } else {
            Ok(Vec::new())
        }
    }
    fn send(&self, _ctx: &pii_core::Ctx, _to: &str, _text: &str) -> Result<(), PiiError> {
        Ok(())
    }
}

fn setup(dir: &std::path::Path, channel: Arc<dyn Channel>) -> (Arc<Engine>, Arc<DaemonContext>) {
    let fixture = dir.join("fixture-empty.json");
    std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: dir.to_path_buf(),
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        mqtt_tool_url: None,
    })
    .expect("engine");
    let net = Arc::new(pii_net::NetService::new(pii_net::NetConfig {
        resolver: Arc::new(pii_net::SystemResolver),
        grants: Arc::new(pii_net::grant::StaticGrants::default()),
        clock: Arc::new(pii_core::SystemClock),
    }));
    let mut channels = std::collections::BTreeMap::new();
    channels.insert(
        String::from(channel.name()),
        ChannelRuntime {
            channel,
            poll_interval_ms: 10,
            graph: String::from("main"),
        },
    );
    let ctx = Arc::new(DaemonContext {
        net,
        workers: Arc::new(parking_lot::Mutex::new(std::collections::BTreeMap::new())),
        channels: parking_lot::Mutex::new(channels),
        channel_store: Arc::new(
            pii_store::channel::ChannelStore::open(&dir.join("channel.redb")).unwrap(),
        ),
        timer_store: Arc::new(pii_store::timer::TimerWheel::open(&dir.join("timer.redb")).unwrap()),
        cron: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        cron_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        cron_last_start: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        channel_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        crashes_dir: dir.join("crashes"),
        config_path: dir.join("config.toml"),
        project_path: None,
        channels_scope: dir.join("channels"),
        runtime_dir: dir.to_path_buf(),
    });
    (engine, ctx)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_429_with_retry_after_paces_the_next_poll() {
    let tmp = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "poll-backoff-").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (engine, ctx) = setup(
        tmp.path(),
        Arc::new(Limited {
            calls: Arc::clone(&calls),
        }),
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);

    tokio::time::sleep(Duration::from_millis(400)).await;
    let early = calls.load(Ordering::SeqCst);
    assert!(
        (1..=3).contains(&early),
        "retry_after=1s must pace the loop; {early} polls in 400ms (unpaced would be >= 20)"
    );

    // The pause is ~1s; after the success the loop is back on its 10ms interval.
    let deadline = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        calls.load(Ordering::SeqCst) >= 2,
        "the second poll must come"
    );
    let at = calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let later = calls.load(Ordering::SeqCst);
    assert!(
        later - at >= 8,
        "a success returns to the interval: only {} polls in 400ms",
        later - at
    );
    for h in handles {
        h.abort();
    }
}
