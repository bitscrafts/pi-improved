//! Channel router (F6.5): poll → start a run → advance the cursor →
//! (reply happens reactively in [`super::finish_run`] when the run
//! terminates).

use std::sync::Arc;
use std::time::Duration;

use pii_core::{Budget, CapKind, CapSet, Ctx, PathScope, Principal, TraceCtx};
use pii_proto::event::Event;

use crate::Engine;
use crate::daemon::status::DaemonContext;

/// Spawns one polling task per configured channel (F6.5). Each task runs
/// until aborted; a channel error is absorbed (F6.3, recorded on the
/// channel cursor) and never kills the daemon.
///
/// The pacing and the state changes are `pii_channels::poll::run_loop` (spec 347).
pub fn spawn_all(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
) -> Vec<tokio::task::JoinHandle<()>> {
    ctx.channels
        .lock()
        .iter()
        .map(|(n, c)| {
            (
                n.clone(),
                c.poll_interval_ms,
                c.channel.kind().to_owned(),
                Arc::clone(&c.backoff),
            )
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(name, interval_ms, kind, backoff)| {
            let engine = Arc::clone(engine);
            let ctx = Arc::clone(ctx);
            tokio::spawn(async move {
                let interval = poll_interval_for(&kind, interval_ms);
                // `poll_once` is fully synchronous (the `Channel` trait
                // is blocking) and a channel poll may block on network
                // or file I/O for a long time. Run it on the blocking
                // thread pool so a slow poll never starves the async
                // executor's worker threads (TDD: poll must not starve
                // concurrent tokio tasks).
                pii_channels::poll::run_loop(
                    interval,
                    &backoff,
                    || {
                        let (eng, ctx2, name2) =
                            (Arc::clone(&engine), Arc::clone(&ctx), name.clone());
                        poll_on_blocking_thread(eng, ctx2, name2)
                    },
                    |ms| tokio::time::sleep(Duration::from_millis(ms)),
                    |change, state| report_change(&engine, &name, change, state),
                )
                .await;
            })
        })
        .collect()
}

/// Runs one poll on the blocking pool. A panic inside the poll is recorded (spec 356) and counts as a failed poll,
/// so the channel backs off and degrades instead of polling on as if healthy.
async fn poll_on_blocking_thread(
    engine: Arc<Engine>,
    ctx: Arc<DaemonContext>,
    name: String,
) -> Result<(), pii_abi::PiiError> {
    let eng = Arc::clone(&engine);
    let polled_name = name.clone();
    match tokio::task::spawn_blocking(move || poll_once(&eng, &ctx, &polled_name)).await {
        Ok(result) => result,
        Err(e) if e.is_panic() => {
            engine.log().event(
                &pii_observe::Event::new(
                    "channel",
                    pii_observe::Level::Error,
                    pii_observe::names::CHANNEL_POLL_PANICKED,
                )
                .field("channel", name.as_str()),
            );
            Err(pii_abi::PiiError::Unavailable(format!(
                "channel '{name}' poll panicked"
            )))
        }
        // Cancelled by runtime shutdown: nothing failed.
        Err(_) => Ok(()),
    }
}

/// The daemon's record of a channel's state change (spec 347): one line when it starts failing and one when it
/// answers again, never one per attempt.
fn report_change(
    engine: &Arc<Engine>,
    name: &str,
    change: pii_channels::poll::Change,
    state: &pii_channels::poll::PollBackoff,
) {
    let (code, level) = match change {
        pii_channels::poll::Change::Degraded => (
            pii_observe::names::CHANNEL_DEGRADED,
            pii_observe::Level::Warn,
        ),
        pii_channels::poll::Change::Recovered => (
            pii_observe::names::CHANNEL_RECOVERED,
            pii_observe::Level::Info,
        ),
    };
    engine.log().event(
        &pii_observe::Event::new("channel", level, code)
            .field("channel", name)
            .field("error", state.last_error().unwrap_or_default())
            .field("failures", state.failures().to_string()),
    );
}

/// The shortest pause a channel may poll at.
///
/// A test channel may poll at 10 ms; Discord may not. Its read route allows about five requests per five
/// seconds, so a configured interval below one second is not a preference — it is a request to be rate
/// limited, and measured 2026-11-02 it was exactly that: 173 `429`s and not one message read.
const DISCORD_MIN_POLL_MS: u64 = 1_000;

pub(crate) fn poll_interval_for(kind: &str, configured_ms: u64) -> u64 {
    if kind == "discord" {
        configured_ms.max(DISCORD_MIN_POLL_MS)
    } else {
        configured_ms.max(10)
    }
}

pub(crate) fn channel_ctx(
    channels_scope: &std::path::Path,
    name: &str,
    egress_hosts: &[String],
) -> Ctx {
    // The channel needs NET (Discord) AND FS (the cli channel's inbox/
    // outbox files are read/written through Ctx::fs). Capability denial is
    // enforced by Ctx::fs/ctx.net; the router grants both.
    //
    // Phase 5.0 D2: the fs scope is `<runtime_dir>/channels/<name>` —
    // derived from the daemon's resolved runtime dir (`channels_scope`,
    // set in `daemon/mod.rs::Daemon::build` from `resolve_runtime_dir`),
    // never a hardcoded literal.
    //
    // Spec 132: the net hosts are exactly what the channel declares through
    // `Channel::egress_hosts`; a channel that declares none can dial nothing.
    let hosts = pii_core::HostScope::new(egress_hosts.to_vec());
    let scope = channels_scope.join(name);
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET | CapKind::FS_READ | CapKind::FS_WRITE;
    caps.fs_read = PathScope::new(vec![scope.clone()]);
    caps.fs_write = PathScope::new(vec![scope.clone()]);
    caps.net_hosts = hosts.clone();
    Ctx::with_observer(
        caps,
        Budget::unlimited(),
        Arc::new(pii_core::NullObserver),
        TraceCtx {
            run: pii_abi::RunId::from("channel"),
            span: 0,
        },
    )
    .child(
        Principal::Channel(name.to_owned()),
        &{
            let mut c = CapSet::empty();
            c.kinds = CapKind::NET | CapKind::FS_READ | CapKind::FS_WRITE;
            c.fs_read = PathScope::new(vec![scope.clone()]);
            c.fs_write = PathScope::new(vec![scope]);
            c.net_hosts = hosts;
            c
        },
        &pii_core::BudgetShare::Shared,
    )
}

fn poll_once(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
    name: &str,
) -> Result<(), pii_abi::PiiError> {
    let Some((channel, graph)) = ctx
        .channels
        .lock()
        .get(name)
        .map(|c| (Arc::clone(&c.channel), c.graph.clone()))
    else {
        return Ok(());
    };
    let cursor = ctx.channel_store.get(name).ok().flatten().map(|c| c.cursor);
    let rctx = channel_ctx(&ctx.channels_scope, name, &channel.egress_hosts());
    let incoming = match channel.poll(&rctx, cursor) {
        Ok(incoming) => incoming,
        Err(e) => {
            let _ = ctx.channel_store.record_error(name);
            return Err(e);
        }
    };
    for msg in incoming {
        let mut input = serde_json::json!({
            "text": msg.text, "chat": msg.chat, "channel": msg.channel, "user": msg.user,
        });
        if channel.kind() == "mqtt" {
            add_mqtt_fields(&mut input, &msg);
        }
        // Spec 305 R4: registered before the run can finish, so its reply lookup cannot miss it.
        let run_id = match engine.start_run_with(&graph, &input, |id| {
            ctx.channel_runs
                .lock()
                .insert(id.to_string(), (name.to_owned(), msg.chat.clone()));
        }) {
            Ok(id) => id,
            Err(err) => {
                engine.log().event(
                    &pii_observe::Event::new(
                        "channel",
                        pii_observe::Level::Error,
                        pii_observe::names::CHANNEL_RUN_REFUSED,
                    )
                    .field("channel", name)
                    .field("graph", graph.as_str())
                    .field("error", err.to_string()),
                );
                continue;
            }
        };
        // Spec 356: a cursor that did not advance re-delivers the message; say so, with the run that holds it.
        if let Err(e) = ctx.channel_store.advance(name, msg.cursor, run_id.as_str()) {
            engine.log().event(
                &pii_observe::Event::new(
                    "channel",
                    pii_observe::Level::Error,
                    pii_observe::names::CURSOR_ADVANCE_FAILED,
                )
                .field("channel", name)
                .field("run", run_id.as_str())
                .field("error", e.to_string()),
            );
        }
        engine.bus.broadcast(Event::ChannelMessage {
            channel: name.to_owned(),
            chat: msg.chat,
            run_id: run_id.to_string(),
        });
    }
    Ok(())
}

/// The MQTT mapping (spec 117 R1a): `topic` is the message topic and `payload`
/// is the payload parsed as JSON when it is valid JSON, else the text itself;
/// `text` keeps the raw payload either way.
fn add_mqtt_fields(input: &mut serde_json::Value, msg: &pii_channels::Incoming) {
    let payload = serde_json::from_str::<serde_json::Value>(&msg.text)
        .unwrap_or_else(|_| serde_json::Value::String(msg.text.clone()));
    input["topic"] = serde_json::Value::String(msg.chat.clone());
    input["payload"] = payload;
}

/// Sends the reply for a finished channel run (F6.5): `Success` sends the
/// output text, `Failure` sends `"error: <message>"`. A send failure is
/// retried once, then dropped — never kills the daemon.
pub fn reply(ctx: &Arc<DaemonContext>, channel: &str, chat: &str, text: &str) {
    let Some(c) = ctx
        .channels
        .lock()
        .get(channel)
        .map(|c| Arc::clone(&c.channel))
    else {
        return;
    };
    let rctx = channel_ctx(&ctx.channels_scope, channel, &c.egress_hosts());
    // A rate limit carries its own pause: a second dial would land in the same closed window.
    if let Err(e) = c.send(&rctx, chat, text)
        && pii_channels::retry_after_ms(&e).is_none()
    {
        let _ = c.send(&rctx, chat, text);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::channel_ctx;
    use pii_channels::poll::pause_ms as poll_pause;

    /// Phase 5.0 D2: the channel fs scope is `<channels_scope>/<name>`,
    /// derived from the daemon's runtime dir — never a hardcoded literal.
    /// A path inside that scope resolves `Allowed` (the write succeeds); a
    /// path outside it resolves `Denied`.
    #[test]
    fn channel_ctx_scope_denies_outside_allows_inside() {
        let tmp_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(tmp_root).expect("mkdir _tmp root");
        let root = tempfile::Builder::new()
            .prefix("channel-scope-")
            .tempdir_in(tmp_root)
            .expect("tempdir");
        // Canonicalize: `channel_ctx`'s scope check compares against the
        // *canonical* path (symlinks/`..` resolved by `canonical_for`), and
        // `tmp_root` is built from `CARGO_MANIFEST_DIR/../../_tmp` which
        // still has `..` components at this point.
        let root_path = dunce::canonicalize(root.path()).expect("canonicalize tempdir");
        let channels_scope = root_path.join("channels");
        std::fs::create_dir_all(&channels_scope).expect("mkdir channels_scope");

        let rctx = channel_ctx(&channels_scope, "cli-test", &[]);

        // Inside: <channels_scope>/cli-test/inbox.jsonl -> Allowed.
        let inside = channels_scope.join("cli-test").join("inbox.jsonl");
        let write_result = rctx.fs().expect("fs cap").write(&inside, b"hello");
        assert!(
            write_result.is_ok(),
            "path inside the configured channel scope must be Allowed: {write_result:?}"
        );
        assert_eq!(std::fs::read(&inside).expect("read back"), b"hello");

        // Outside: a sibling directory not under <channels_scope>/cli-test -> Denied.
        let outside = root.path().join("other-channel").join("inbox.jsonl");
        let denied_result = rctx.fs().expect("fs cap").write(&outside, b"nope");
        assert!(
            matches!(denied_result, Err(pii_abi::PiiError::Denied(_))),
            "path outside the configured channel scope must be Denied: {denied_result:?}"
        );
    }

    /// Spec 132: the grant is exactly what the channel declares. Discord's real API is reachable,
    /// a near-miss host is not, and a channel that declares nothing (`cli`) can dial nothing.
    #[test]
    fn channel_ctx_grants_exactly_the_declared_hosts() {
        use pii_channels::Channel;
        let scope = std::path::Path::new("/nonexistent-channels");
        let discord = channel_ctx(scope, "d", &[String::from("discord.com:443")]);
        let hosts = &discord.caps().net_hosts;
        assert!(hosts.permits("discord.com:443"));
        assert!(!hosts.permits("discord.com.evil.test:443"), "near miss");
        assert!(!hosts.permits("api.telegram.org:443"));

        let cli = pii_channels::cli::CliChannel::new(
            String::from("c"),
            scope.join("in"),
            scope.join("out"),
        );
        assert_eq!(cli.egress_hosts(), [] as [String; 0]);
        let rctx = channel_ctx(scope, "c", &cli.egress_hosts());
        assert!(!rctx.caps().net_hosts.permits("discord.com:443"));
        assert!(!rctx.caps().net_hosts.permits(""));
    }

    #[test]
    fn an_error_without_retry_after_uses_the_capped_backoff() {
        let want = [1000, 2000, 4000, 8000, 16000, 32000];
        for (i, w) in want.iter().enumerate() {
            let c = u32::try_from(i).unwrap() + 1;
            assert_eq!(poll_pause(Some("unavailable: boom"), c, 1000), *w, "c={c}");
        }
        for c in 1..=20 {
            assert!(poll_pause(Some("x"), c, 1000) <= 300_000);
            assert!(poll_pause(Some("x"), c, 200_000) <= 300_000);
            assert!(poll_pause(Some(r#""retry_after":600"#), c, 1000) <= 300_000);
        }
    }

    #[test]
    fn a_success_returns_to_the_interval() {
        assert_eq!(poll_pause(Some("x"), 7, 1200), 64 * 1000);
        assert_eq!(poll_pause(None, 0, 1200), 1200);
    }

    #[test]
    fn retry_after_wins_over_the_backoff_and_never_undercuts_the_interval() {
        let t = r#"discord: poll status 429: {"retry_after":132.176}"#;
        assert_eq!(poll_pause(Some(t), 1, 1200), 132_176);
        assert_eq!(poll_pause(Some(r#""retry_after":0.001"#), 1, 1200), 1200);
    }

    #[test]
    fn a_discord_poll_cannot_be_configured_below_the_provider_limit() {
        // The defect, measured on production 2026-11-02: 500 ms against a ~1 req/s route.
        assert_eq!(super::poll_interval_for("discord", 500), 1_000);
        assert_eq!(super::poll_interval_for("discord", 20), 1_000);
        // an operator asking for more than the floor keeps it
        assert_eq!(super::poll_interval_for("discord", 5_000), 5_000);
        // CONTROL: the floor is Discord's, not a general slowdown — a test channel still polls at 10 ms.
        assert_eq!(super::poll_interval_for("cli", 20), 20);
        assert_eq!(super::poll_interval_for("mqtt", 100), 100);
        assert_eq!(super::poll_interval_for("webhook", 1), 10);
    }

    #[test]
    fn a_repeating_429_grows_the_pause_instead_of_repeating_it() {
        // retry_after 0.3 s against a 1500 ms interval: the first answer is the interval, and every repeat
        // after it must wait longer, so a provider that keeps refusing is not answered in kind forever.
        let t = r#"discord: poll status 429: {"retry_after":0.3}"#;
        let got: Vec<u64> = (1..=5).map(|c| poll_pause(Some(t), c, 1_500)).collect();
        assert_eq!(got, vec![1_500, 1_500, 1_500, 2_400, 4_800], "{got:?}");
        // CONTROL: the very same text does NOT slow a first failure below the interval
        assert_eq!(poll_pause(Some(t), 1, 1_500), 1_500);
    }

    #[test]
    fn the_pre_fix_error_text_produces_no_pace() {
        assert_eq!(
            poll_pause(Some("unavailable: discord: poll status 429"), 1, 1200),
            1200
        );
    }

    // ---- poll_on_blocking_thread (TDD: poll must not starve tokio) ----

    use super::spawn_all;
    use std::sync::Arc;
    use std::time::Duration;

    /// A test-only channel whose `poll` blocks for 300ms (simulating slow
    /// network/file I/O) and returns no messages.
    struct SlowChannel {
        name: &'static str,
    }

    impl pii_channels::Channel for SlowChannel {
        fn name(&self) -> &str {
            self.name
        }
        fn kind(&self) -> &'static str {
            "slow-test"
        }
        fn poll(
            &self,
            _ctx: &pii_core::Ctx,
            _cursor: Option<u64>,
        ) -> Result<Vec<pii_channels::Incoming>, pii_abi::PiiError> {
            std::thread::sleep(Duration::from_millis(300));
            Ok(Vec::new())
        }
        fn send(
            &self,
            _ctx: &pii_core::Ctx,
            _to: &str,
            _text: &str,
        ) -> Result<(), pii_abi::PiiError> {
            Ok(())
        }
    }

    /// A minimal `DaemonContext` for the router test (empty workers/cron,
    /// a scratch channel store).
    fn router_ctx(
        channel: Arc<dyn pii_channels::Channel>,
    ) -> Arc<crate::daemon::status::DaemonContext> {
        let tmp_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(tmp_root.join("router-test")).expect("mkdir _tmp/router-test");
        let store = Arc::new(
            pii_store::channel::ChannelStore::open(&tmp_root.join("router-test/channel.redb"))
                .expect("channel store"),
        );
        let net = Arc::new(pii_net::NetService::new(pii_net::NetConfig {
            resolver: Arc::new(pii_net::SystemResolver),
            grants: Arc::new(pii_net::grant::StaticGrants::default()),
            clock: Arc::new(pii_core::SystemClock),
        }));
        let mut channels = std::collections::BTreeMap::new();
        channels.insert(
            String::from(channel.name()),
            crate::daemon::status::ChannelRuntime {
                channel,
                poll_interval_ms: 10,
                graph: String::from("main"),
                backoff: Arc::default(),
            },
        );
        Arc::new(crate::daemon::status::DaemonContext {
            net,
            workers: Arc::new(parking_lot::Mutex::new(std::collections::BTreeMap::new())),
            channels: parking_lot::Mutex::new(channels),
            channel_store: store,
            timer_store: Arc::new(
                pii_store::timer::TimerWheel::open(&tmp_root.join("router-test/timer.redb"))
                    .unwrap(),
            ),
            cron: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            cron_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            cron_last_start: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            clock: Arc::new(pii_core::SystemClock),
            channel_runs: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            crashes_dir: tmp_root.join("router-test/crashes"),
            config_path: tmp_root.join("router-test/config.toml"),
            project_path: None,
            channels_scope: tmp_root.join("router-test/channels"),
            runtime_dir: tmp_root.join("router-test"),
            durable_dir: tmp_root.join("router-test"),
        })
    }

    /// While a channel poll blocks for 300ms, a concurrently-scheduled
    /// lightweight tokio task must still run (~every 10ms). On a
    /// current-thread runtime the old inline `poll_once` starved the whole
    /// executor; `spawn_blocking` keeps it responsive.
    #[tokio::test(flavor = "current_thread")]
    async fn poll_once_runs_on_blocking_thread() {
        // A bare engine with a scripted (empty) model; the poll never starts
        // a run (the slow channel returns no messages), so only the router
        // path is exercised.
        let tmp_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        let dir = tmp_root.join("router-test");
        std::fs::create_dir_all(&dir).expect("mkdir router-test");
        let fixture = dir.join("fixture-empty.json");
        std::fs::write(&fixture, r#"{"turns":[]}"#).expect("write fixture");
        let engine = crate::Engine::new(crate::EngineConfig {
            grant_ceiling: None,
            workspace: dir.clone(),
            store_path: dir.join(format!("store-{}.redb", std::process::id())),
            model: format!("scripted:{}", fixture.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            graph_dir: dir.clone(),
            route_policy: std::collections::BTreeMap::new(),
            tiers: Vec::new(),
            session_runs_cap: crate::DEFAULT_SESSION_RUNS_CAP,
            llama_host: None,
            models: crate::daemon::empty_model_roles(),
            mqtt_tool_url: None,
        })
        .expect("engine");
        let ctx = router_ctx(Arc::new(SlowChannel { name: "slow" }));
        let _handles = spawn_all(&engine, &ctx);

        // A heartbeat task that ticks every 10ms while the poll blocks.
        let ticks = Arc::new(parking_lot::Mutex::new(0u32));
        let t = Arc::clone(&ticks);
        let heartbeat = tokio::spawn(async move {
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                *t.lock() += 1;
            }
        });
        heartbeat.await.expect("heartbeat");
        let n = *ticks.lock();
        assert!(
            n >= 20,
            "tokio executor was starved by the channel poll: only {n}/30 ticks ran"
        );
    }
}
