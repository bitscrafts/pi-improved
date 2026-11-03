//! Spec 347: a failing channel backs off, a recovered one is noticed, the outage is visible on the daemon's own
//! health answer, and the incident carries `resolved_at`. Every test drives the daemon's own `spawn_all` loop on
//! the paused tokio clock, so the waits below are virtual and nothing sleeps for real.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use pii_abi::PiiError;
use pii_channels::observe::ObservedChannel;
use pii_channels::{Channel, Incoming};
use pii_observe::{Recorder, names};
use pii_proto::sys::StatusResult;
use pii_server::daemon::status::{ChannelRuntime, DaemonContext};
use pii_server::{Engine, EngineConfig};
use pii_tools::sandbox::SandboxProfileName;

const OUTAGE: &str = "unavailable: net: write: Connection reset by peer (os error 104)";

/// Fails every poll while `down` is set; counts every poll it is asked for.
struct Flaky {
    down: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
}

impl Channel for Flaky {
    fn name(&self) -> &'static str {
        "flaky"
    }
    fn kind(&self) -> &'static str {
        "flaky-test"
    }
    fn poll(&self, _ctx: &pii_core::Ctx, _cursor: Option<u64>) -> Result<Vec<Incoming>, PiiError> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if self.down.load(Ordering::SeqCst) {
            Err(PiiError::Unavailable(String::from(OUTAGE)))
        } else {
            Ok(Vec::new())
        }
    }
    fn send(&self, _ctx: &pii_core::Ctx, _to: &str, _text: &str) -> Result<(), PiiError> {
        Ok(())
    }
}

/// The daemon as `pii daemon` builds it: the channel wrapped by `ObservedChannel`, the log attached.
fn setup(
    dir: &std::path::Path,
    channel: Arc<dyn Channel>,
    log: Arc<dyn pii_observe::Observer>,
) -> (Arc<Engine>, Arc<DaemonContext>) {
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
    engine.attach_log(Arc::clone(&log));
    let net = Arc::new(pii_net::NetService::new(pii_net::NetConfig {
        resolver: Arc::new(pii_net::SystemResolver),
        grants: Arc::new(pii_net::grant::StaticGrants::default()),
        clock: Arc::new(pii_core::SystemClock),
    }));
    let observed: Arc<dyn Channel> = Arc::new(ObservedChannel::new(channel, log));
    let mut channels = std::collections::BTreeMap::new();
    channels.insert(
        String::from(observed.name()),
        ChannelRuntime {
            channel: observed,
            poll_interval_ms: 10,
            graph: String::from("main"),
            backoff: Arc::new(parking_lot::Mutex::new(
                pii_channels::poll::PollBackoff::new(),
            )),
        },
    );
    let ctx = Arc::new(DaemonContext {
        net,
        workers: Arc::new(parking_lot::Mutex::new(std::collections::BTreeMap::new())),
        clock: Arc::new(pii_core::SystemClock),
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
        durable_dir: dir.to_path_buf(),
    });
    (engine, ctx)
}

/// The answer `pii health` renders, built by the daemon's own status path.
fn health_answer(engine: &Engine, ctx: &DaemonContext) -> StatusResult {
    let mut out = StatusResult::default();
    pii_server::daemon::identity::attach(engine, Some(ctx), &mut out);
    out
}

#[tokio::test(start_paused = true)]
async fn a_failing_poll_backs_off() {
    let dir = tempfile::tempdir().unwrap();
    let polls = Arc::new(AtomicUsize::new(0));
    let (engine, ctx) = setup(
        dir.path(),
        Arc::new(Flaky {
            down: Arc::new(AtomicBool::new(true)),
            polls: Arc::clone(&polls),
        }),
        Arc::new(Recorder::new()),
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    tokio::time::sleep(Duration::from_secs(60)).await;
    let attempts = polls.load(Ordering::SeqCst);
    for h in handles {
        h.abort();
    }
    assert!(
        attempts <= 20,
        "a failing channel was polled {attempts} times in 60 s; it must back off"
    );
}

#[tokio::test(start_paused = true)]
#[allow(non_snake_case)] // reason: the spec names this test with the CONTROL_ prefix
async fn CONTROL_a_healthy_poll_keeps_its_cadence() {
    let dir = tempfile::tempdir().unwrap();
    let polls = Arc::new(AtomicUsize::new(0));
    let (engine, ctx) = setup(
        dir.path(),
        Arc::new(Flaky {
            down: Arc::new(AtomicBool::new(false)),
            polls: Arc::clone(&polls),
        }),
        Arc::new(Recorder::new()),
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let healthy = polls.load(Ordering::SeqCst);
    for h in handles {
        h.abort();
    }
    assert!(
        healthy >= 50,
        "a healthy channel on a 10 ms interval was polled {healthy} times in 1 s"
    );
}

#[tokio::test(start_paused = true)]
async fn a_success_after_failures_resets_the_interval() {
    let dir = tempfile::tempdir().unwrap();
    let down = Arc::new(AtomicBool::new(true));
    let polls = Arc::new(AtomicUsize::new(0));
    let (engine, ctx) = setup(
        dir.path(),
        Arc::new(Flaky {
            down: Arc::clone(&down),
            polls: Arc::clone(&polls),
        }),
        Arc::new(Recorder::new()),
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    // Five failures back the loop off to a pause of 16 s; the next attempt, at t=31 s, succeeds.
    tokio::time::sleep(Duration::from_secs(20)).await;
    down.store(false, Ordering::SeqCst);
    // long enough for the next attempt (a few seconds at most with a handful of failures) to succeed
    tokio::time::sleep(Duration::from_secs(15)).await;
    let before = polls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = polls.load(Ordering::SeqCst);
    for h in handles {
        h.abort();
    }
    assert!(
        after - before >= 50,
        "after recovery the loop must return to its interval: {} polls in 1 s",
        after - before
    );
}

#[tokio::test(start_paused = true)]
async fn health_reports_a_degraded_channel() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, ctx) = setup(
        dir.path(),
        Arc::new(Flaky {
            down: Arc::new(AtomicBool::new(true)),
            polls: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(Recorder::new()),
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    tokio::time::sleep(Duration::from_secs(5)).await;
    let answer = health_answer(&engine, &ctx);
    for h in handles {
        h.abort();
    }
    let sub = answer
        .subsystems
        .iter()
        .find(|s| s.name == "channel.flaky")
        .expect("the channel is a subsystem of the health answer");
    assert_eq!(sub.state, "degraded", "{sub:?}");
    let detail = sub.detail.as_deref().unwrap_or_default();
    assert!(detail.contains("Connection reset by peer"), "{detail}");
    assert!(detail.contains("count="), "{detail}");
    assert!(detail.contains("last attempt"), "{detail}");
}

#[tokio::test(start_paused = true)]
async fn the_state_change_is_logged_once_not_per_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let down = Arc::new(AtomicBool::new(true));
    let log = Arc::new(Recorder::new());
    let (engine, ctx) = setup(
        dir.path(),
        Arc::new(Flaky {
            down: Arc::clone(&down),
            polls: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::clone(&log) as Arc<dyn pii_observe::Observer>,
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    tokio::time::sleep(Duration::from_secs(20)).await;
    down.store(false, Ordering::SeqCst);
    // long enough for the next attempt (a few seconds at most with a handful of failures) to succeed
    tokio::time::sleep(Duration::from_secs(15)).await;
    for h in handles {
        h.abort();
    }
    assert!(
        log.events_named(names::CHANNEL_POLL_FAILED).len() > 1,
        "the attempts are real: several failed polls must have happened"
    );
    assert_eq!(log.events_named(names::CHANNEL_DEGRADED).len(), 1);
    assert_eq!(log.events_named(names::CHANNEL_RECOVERED).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_incident_carries_resolved_at_after_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let down = Arc::new(AtomicBool::new(true));
    let log_path = dir.path().join("daemon.log");
    let file_log = Arc::new(pii_observe::FileLog::new(&log_path, 0, 1));
    let (engine, ctx) = setup(
        dir.path(),
        Arc::new(Flaky {
            down: Arc::clone(&down),
            polls: Arc::new(AtomicUsize::new(0)),
        }),
        file_log,
    );
    let handles = pii_server::daemon::router::spawn_all(&engine, &ctx);
    // TWO failures, not five seconds of them. The channel polls every 10 ms, so five seconds is thousands of
    // attempts: the backoff saturates at its 5-minute ceiling and the next attempt lands far outside any
    // affordable wait, which is CORRECT behaviour and an impossible test. At ~150 ms there are a handful of
    // failures, the wait is a couple of seconds, and the recovery is observable.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let config = pii_learn::incident::IncidentConfig {
        threshold: 1,
        window_secs: 3600,
        benign: Vec::new(),
    };
    let during = pii_learn::incident_runtime::scan_runtime_log(&log_path, &config).unwrap();
    let failing = during
        .iter()
        .find(|i| i.fingerprint == "refusal-repeated:channel.poll.failed")
        .expect("the outage opens an incident");
    assert!(failing.last_seen > 0, "{failing:?}");
    assert_eq!(failing.resolved_at, None, "still failing: {failing:?}");

    down.store(false, Ordering::SeqCst);
    // long enough for the next attempt (a few seconds at most with a handful of failures) to succeed
    tokio::time::sleep(Duration::from_secs(15)).await;
    for h in handles {
        h.abort();
    }
    let after = pii_learn::incident_runtime::scan_runtime_log(&log_path, &config).unwrap();
    let recovered = after
        .iter()
        .find(|i| i.fingerprint == "refusal-repeated:channel.poll.failed")
        .expect("the outage incident is still reported");
    assert!(recovered.resolved_at.is_some(), "{recovered:?}");
    assert!(
        recovered.resolved_at >= Some(recovered.last_seen),
        "{recovered:?}"
    );
}
