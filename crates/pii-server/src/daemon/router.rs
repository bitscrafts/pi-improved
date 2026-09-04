//! Channel router (F6.5): poll → start a run → advance the cursor →
//! (reply happens reactively in [`super::finish_run`] when the run
//! terminates).

use std::sync::Arc;
use std::time::Duration;

use pii_core::{Budget, CapKind, CapSet, Ctx, Principal, TraceCtx};
use pii_proto::event::Event;

use crate::Engine;
use crate::daemon::status::DaemonContext;

/// Spawns one polling task per configured channel (F6.5). Each task runs
/// until aborted; a channel error is absorbed (F6.3, recorded on the
/// channel cursor) and never kills the daemon.
pub fn spawn_all(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
) -> Vec<tokio::task::JoinHandle<()>> {
    ctx.channels
        .lock()
        .iter()
        .map(|(n, c)| (n.clone(), c.poll_interval_ms))
        .collect::<Vec<(String, u64)>>()
        .into_iter()
        .map(|(name, interval_ms)| {
            let engine = Arc::clone(engine);
            let ctx = Arc::clone(ctx);
            tokio::spawn(async move {
                loop {
                    poll_once(&engine, &ctx, &name);
                    tokio::time::sleep(Duration::from_millis(interval_ms.max(10))).await;
                }
            })
        })
        .collect()
}

fn channel_ctx(name: &str) -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
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
            c.kinds = CapKind::NET;
            c
        },
        &pii_core::BudgetShare::Shared,
    )
}

fn poll_once(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>, name: &str) {
    let Some((channel, graph)) = ctx
        .channels
        .lock()
        .get(name)
        .map(|c| (Arc::clone(&c.channel), c.graph.clone()))
    else {
        return;
    };
    let cursor = ctx.channel_store.get(name).ok().flatten().map(|c| c.cursor);
    let rctx = channel_ctx(name);
    let Ok(incoming) = channel.poll(&rctx, cursor) else {
        let _ = ctx.channel_store.record_error(name);
        return;
    };
    for msg in incoming {
        let input = serde_json::json!({
            "text": msg.text, "chat": msg.chat, "channel": msg.channel, "user": msg.user,
        });
        let Ok(run_id) = engine.start_run(&graph, &input) else {
            continue;
        };
        let _ = ctx.channel_store.advance(name, msg.cursor, run_id.as_str());
        ctx.channel_runs
            .lock()
            .insert(run_id.to_string(), (name.to_owned(), msg.chat.clone()));
        engine.bus.broadcast(Event::ChannelMessage {
            channel: name.to_owned(),
            chat: msg.chat,
            run_id: run_id.to_string(),
        });
    }
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
    let rctx = channel_ctx(channel);
    if c.send(&rctx, chat, text).is_err() {
        let _ = c.send(&rctx, chat, text);
    }
}
