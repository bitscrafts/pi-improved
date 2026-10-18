//! Cron re-arming and terminal-run reactions of the reactive loop.

use std::sync::Arc;

use pii_proto::event::WireTerminal;

use super::router;
use super::status::{CronEntry, DaemonContext};
use crate::Engine;

/// Minimum delay before a finished cron run is re-armed: a graph that finishes at
/// once (or a start that keeps failing) must not re-arm in a hot loop (spec 177).
const CRON_MIN_REARM: std::time::Duration = std::time::Duration::from_secs(1);

/// Starts one cron firing; a refusal is a `cron.start.refused` event (spec 177).
pub(super) fn start_cron(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>, e: &CronEntry) {
    ctx.cron_last_start
        .lock()
        .insert(e.name.clone(), std::time::Instant::now());
    match engine.start_run(&e.graph, &e.input) {
        Ok(run_id) => {
            ctx.cron_runs
                .lock()
                .insert(run_id.to_string(), e.name.clone());
        }
        Err(err) => engine.log().event(
            &pii_observe::Event::new(
                "cron",
                pii_observe::Level::Error,
                pii_observe::names::CRON_START_REFUSED,
            )
            .field("cron", e.name.as_str())
            .field("graph", e.graph.as_str())
            .field("error", err.to_string()),
        ),
    }
}

/// Re-arms a cron entry whose run finished: at once when its last firing started at least
/// [`CRON_MIN_REARM`] ago (spec 183), else after the remainder of that interval.
fn rearm_cron(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>, entry: CronEntry) {
    let wait = ctx
        .cron_last_start
        .lock()
        .get(&entry.name)
        .map_or(std::time::Duration::ZERO, |t| {
            CRON_MIN_REARM.saturating_sub(t.elapsed())
        });
    if wait.is_zero() {
        start_cron(engine, ctx, &entry);
        return;
    }
    let (engine, ctx) = (Arc::clone(engine), Arc::clone(ctx));
    // Spec 178: the re-arm thread has no runtime of its own; enter the reactive loop's.
    let rt = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let _guard = rt.enter();
        std::thread::sleep(wait);
        start_cron(&engine, &ctx, &entry);
    });
}

/// Reacts to a terminal run (F5.8/F6.5): starts the next cron firing, or
/// replies on the channel that started this run.
pub(super) fn on_finished(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
    run_id: &str,
    terminal: &WireTerminal,
) {
    // Spec 179: a run parked on its timer journals `Finished{Suspended}`; it is not over, and
    // re-arming here started a new run every CRON_MIN_REARM. Stay tracked until it truly ends.
    let parked = matches!(terminal, WireTerminal::Suspended { .. });
    let cron_name = if parked {
        None
    } else {
        ctx.cron_runs.lock().remove(run_id)
    };
    if let Some(name) = cron_name {
        let entry = ctx.cron.lock().get(&name).cloned();
        if let Some(entry) = entry {
            rearm_cron(engine, ctx, entry);
        }
    }
    let channel_run = ctx.channel_runs.lock().remove(run_id);
    if let Some((channel, chat)) = channel_run {
        let text = match terminal {
            WireTerminal::Success { output } => output
                .get("text")
                .and_then(|v| v.as_str())
                .map_or_else(|| output.to_string(), str::to_owned),
            WireTerminal::Failure { error } => format!("error: {error}"),
            WireTerminal::Suspended { .. } => return,
        };
        // Spec 128: a graph that already answered through a `post` node ends with `text = ""`;
        // an empty reply is never sent (Discord refuses empty content, and it would be noise).
        if text.is_empty() {
            return;
        }
        router::reply(ctx, &channel, &chat, &text);
    }
}
