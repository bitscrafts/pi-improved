//! Spec 356: the paths that wrote nothing now write one named event each. Every test drives the real path (the failing
//! wheel, the claim through dispatch, the panicking poll) and reads the event back from the journal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pii_abi::PiiError;
use pii_channels::observe::ObservedChannel;
use pii_channels::{Channel, Incoming};
use pii_core::SystemClock;
use pii_observe::{Recorder, names};
use pii_proto::{Id, Request, method::Method};
use pii_server::daemon::scheduler::{Scheduler, TimerSource};
use pii_server::daemon::status::{ChannelRuntime, DaemonContext};
use pii_server::engine::conn::{PeerIdentity, SubscriptionRegistry};
use pii_server::engine::dispatch::dispatch;
use pii_server::{Engine, EngineConfig};
use pii_store::timer::{DueTimer, TimerWheel};
use pii_tools::sandbox::SandboxProfileName;

fn hello() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("fixtures/phase2/hello.json")
}

fn engine_in(dir: &Path) -> Arc<Engine> {
    Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.to_path_buf(),
        store_path: dir.join("store.redb"),
        model: format!("scripted:{}", hello().display()),
        sandbox: SandboxProfileName::Off,
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        graph_dir: dir.to_path_buf(),
        mqtt_tool_url: None,
    })
    .expect("engine")
}

/// A wheel whose every read fails, as a damaged redb file would.
struct BrokenWheel;

impl TimerSource for BrokenWheel {
    fn next_timer(&self) -> Result<Option<u64>, PiiError> {
        Err(PiiError::Internal(String::from("wheel read failed")))
    }
    fn take_due(&self, _now_ms: u64) -> Result<Vec<DueTimer>, PiiError> {
        Ok(Vec::new())
    }
    fn put_timer(&self, _fire_at_ms: u64, _run: &str, _key: &str) -> Result<(), PiiError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_timer_error_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let sched = Scheduler::new(engine, Arc::new(BrokenWheel), Arc::new(SystemClock));
    let handle = tokio::spawn(Arc::clone(&sched).run());

    // Several backoff periods pass; the outage is still reported once.
    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.abort();

    assert_eq!(
        log.events_named(names::TIMER_WHEEL_ERROR).len(),
        1,
        "a failing wheel read must produce exactly one timer.error"
    );
}

#[test]
fn a_claimed_item_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_in(tmp.path());
    engine
        .queue
        .put_item(&pii_store::queue::WorkItem::new("A1", &[]))
        .unwrap();
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());

    let (wtx, _wrx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let req = Request {
        jsonrpc: String::from("2.0"),
        id: Id::Num(1),
        method: Method::QueueClaim,
        params: serde_json::json!({"by": "r-claimer"}),
    };
    let resp = dispatch(
        &engine,
        &req,
        PeerIdentity::InProcess,
        &wtx,
        &SubscriptionRegistry::default(),
    );
    assert!(resp.error.is_none(), "the claim must succeed: {resp:?}");

    let claimed = log.events_named(names::QUEUE_ITEM_CLAIMED);
    assert_eq!(claimed.len(), 1, "one claim, one queue.item.claimed");
    assert_eq!(
        claimed
            .first()
            .and_then(|e| e.fields.get("item"))
            .and_then(|v| v.as_str()),
        Some("A1"),
        "the event carries the item id"
    );
}

#[tokio::test]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
async fn CONTROL_a_successful_timer_records_fired_and_no_error() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_in(tmp.path());
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let wheel = Arc::new(TimerWheel::open(&tmp.path().join("timer.redb")).unwrap());
    wheel.put_timer(0, "r-fired", "timer:r-fired:t:0").unwrap();
    let sched = Scheduler::new(engine, wheel, Arc::new(SystemClock));
    let handle = tokio::spawn(Arc::clone(&sched).run());

    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.abort();

    assert_eq!(log.events_named(names::TIMER_FIRED).len(), 1);
    assert!(
        log.events_named(names::TIMER_WHEEL_ERROR).is_empty(),
        "a healthy wheel must not record a timer.error"
    );
}

/// A channel whose poll panics, as a buggy adapter would.
struct PanickingPoll {
    armed: Arc<AtomicBool>,
}

impl Channel for PanickingPoll {
    fn name(&self) -> &'static str {
        "panicky"
    }
    fn kind(&self) -> &'static str {
        "panicky-test"
    }
    fn poll(&self, _ctx: &pii_core::Ctx, _cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        assert!(!self.armed.load(Ordering::SeqCst), "poll adapter bug");
        Ok(Vec::new())
    }
    fn send(&self, _ctx: &pii_core::Ctx, _to: &str, _text: &str) -> Result<(), PiiError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_panicking_poll_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("fixture-empty.json");
    std::fs::write(&fixture, r#"{"turns":[]}"#).unwrap();
    let engine = Engine::new(EngineConfig {
        grant_ceiling: None,
        workspace: dir.path().to_path_buf(),
        store_path: dir.path().join("store.redb"),
        model: format!("scripted:{}", fixture.display()),
        sandbox: SandboxProfileName::Off,
        graph_dir: dir.path().to_path_buf(),
        route_policy: std::collections::BTreeMap::new(),
        tiers: Vec::new(),
        session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
        llama_host: None,
        mqtt_tool_url: None,
    })
    .expect("engine");
    let log = Arc::new(Recorder::new());
    engine.attach_log(log.clone());
    let net = Arc::new(pii_net::NetService::new(pii_net::NetConfig {
        resolver: Arc::new(pii_net::SystemResolver),
        grants: Arc::new(pii_net::grant::StaticGrants::default()),
        clock: Arc::new(SystemClock),
    }));
    let inner: Arc<dyn Channel> = Arc::new(PanickingPoll {
        armed: Arc::new(AtomicBool::new(true)),
    });
    let observed: Arc<dyn Channel> = Arc::new(ObservedChannel::new(inner, log.clone()));
    let backoff = Arc::new(parking_lot::Mutex::new(
        pii_channels::poll::PollBackoff::new(),
    ));
    let mut channels = std::collections::BTreeMap::new();
    channels.insert(
        String::from(observed.name()),
        ChannelRuntime {
            channel: observed,
            poll_interval_ms: 10,
            graph: String::from("main"),
            backoff: Arc::clone(&backoff),
        },
    );
    let ctx = Arc::new(DaemonContext {
        net,
        workers: Arc::new(parking_lot::Mutex::new(std::collections::BTreeMap::new())),
        clock: Arc::new(SystemClock),
        channels: parking_lot::Mutex::new(channels),
        channel_store: Arc::new(
            pii_store::channel::ChannelStore::open(&dir.path().join("channel.redb")).unwrap(),
        ),
        timer_store: Arc::new(TimerWheel::open(&dir.path().join("timer.redb")).unwrap()),
        cron: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        cron_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        cron_last_start: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        channel_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        crashes_dir: dir.path().join("crashes"),
        config_path: dir.path().join("config.toml"),
        project_path: None,
        channels_scope: dir.path().join("channels"),
        runtime_dir: dir.path().to_path_buf(),
        durable_dir: dir.path().to_path_buf(),
    });
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    for h in handles {
        h.abort();
    }

    assert!(
        !log.events_named(names::CHANNEL_POLL_PANICKED).is_empty(),
        "a panicking poll must be recorded as channel.poll.panicked"
    );
    assert!(
        backoff.lock().failures() >= 1,
        "a panicking poll is a failed poll: the channel must degrade"
    );
    assert!(
        !log.events_named(names::CHANNEL_DEGRADED).is_empty(),
        "the degraded channel must be reported"
    );
}
