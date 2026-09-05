//! `event.subscribe`/`event.unsubscribe` (F2.5, M1 opus review).
//!
//! Split out of `dispatch.rs` to stay under the 450-effective-line lint
//! cap after M1 added real subscription cancellation (a `SubscriptionRegistry`
//! handle per subscriber task, instead of the previous ack-only no-op).

use std::sync::Arc;

use tokio::sync::mpsc;

use pii_abi::{PiiError, RunId};
use pii_proto::event::{Event, EventFilter, EventSubscribeResult};
use pii_proto::method::Method;
use pii_proto::run::SeqEvent;
use pii_proto::{Envelope, Notification, Request, Response, write_frame};

use crate::Engine;
use crate::bus::reattach;
use crate::conn::SubscriptionRegistry;

/// Advertised in `sys.hello`'s `capabilities`/limits (F1.10) and enforced
/// here (M1, opus review): a connection offering an unbounded number of
/// live subscriptions is an unbounded number of tasks + bounded queues, so
/// the cap is a real ceiling, not documentation.
pub const MAX_SUBSCRIPTIONS_PER_CONN: usize = 32;

/// `event.subscribe` (F2.5): replay `seq > since`, then attach live with
/// dedup. The live forwarder filters by the subscription's run and kinds
/// (F3) and uses a bounded per-subscription queue that emits `Lagged`
/// instead of dropping silently (F4).
// reason: the forwarder is a single select! loop; splitting it would obscure
// the replay/live/dedup/lag handling.
#[allow(clippy::too_many_lines)]
pub(crate) fn subscribe(
    engine: &Arc<Engine>,
    req: &Request,
    wtx: &mpsc::UnboundedSender<String>,
    subs: &SubscriptionRegistry,
) -> Result<Response, PiiError> {
    if subs.len() >= MAX_SUBSCRIPTIONS_PER_CONN {
        return Err(PiiError::Invalid(format!(
            "event.subscribe: connection already has {MAX_SUBSCRIPTIONS_PER_CONN} subscriptions"
        )));
    }
    let filter: EventFilter = serde_json::from_value(req.params.clone())
        .map_err(|_| PiiError::Invalid(String::from("bad event.subscribe params")))?;
    let sub_id = format!("sub-{}", engine.started_at.elapsed().as_nanos());
    // Resolve the target run: an explicit `run` wins; otherwise a `session`
    // subscribes to that session's most recent run (F2.5). Neither → invalid.
    let run_id = match (&filter.run, &filter.session) {
        (Some(r), _) => RunId::from(r.as_str()),
        (None, Some(s)) => {
            let runs = engine.session_run_ids(s);
            let latest = runs
                .last()
                .ok_or_else(|| PiiError::Invalid(String::from("session has no runs")))?;
            RunId::from(latest.as_str())
        }
        (None, None) => {
            return Err(PiiError::Invalid(String::from(
                "event.subscribe needs run or session",
            )));
        }
    };
    let since = filter.since_seq.unwrap_or(0);
    let kinds = filter.kinds;
    let run_id_str = run_id.to_string();
    let mut live = engine.bus.subscribe();
    let mut merged: Vec<SeqEvent> = Vec::new();
    let mut last = reattach(&engine.store, &run_id, since, &mut live, &mut merged);
    let result = Response::ok(
        req.id.clone(),
        serde_json::to_value(EventSubscribeResult {
            sub_id: sub_id.clone(),
        })
        .unwrap_or(serde_json::Value::Null),
    );

    // Per-subscription bounded queue (F4): drop-oldest + Lagged, never silent.
    let (sub_tx, mut sub_rx) = mpsc::channel::<String>(1024);
    // Replay first (in order), then forward live with dedup + filtering.
    for se in merged {
        let _ = sub_tx.try_send(event_line(&sub_id, &se));
    }
    let wtx2 = wtx.clone();
    let sub_id2 = sub_id.clone();
    // M1 (opus review): a cancellation the connection loop can trigger from
    // `event.unsubscribe` or its own teardown — the previous `select!` only
    // exited when it had something to send, so an idle subscription (or one
    // whose client vanished) leaked this task + its bounded queue forever.
    // A `oneshot` is enough (no new dependency): the sender is held by the
    // registry, the receiver is raced in the loop's `select!`.
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let mut dropped: u64 = 0;
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                // Drain the bounded queue to the connection writer.
                out = sub_rx.recv() => {
                    match out {
                        Some(l) => { if wtx2.send(l).is_err() { break; } }
                        None => break,
                    }
                }
                // Read the next live bus event.
                se = live.recv() => {
                    match se {
                        Ok(se) => {
                            // F3: filter by the subscription's run and kinds.
                            if !event_matches_run(&se.event, &run_id_str) { continue; }
                            if !event_matches_kinds(&se.event, &kinds) { continue; }
                            // Live-only events (ToolCall/ToolResult/AssistantDelta)
                            // carry seq 0 and must always be forwarded (F12);
                            // journalled events are deduped by seq.
                            if !is_live_only(&se.event) {
                                if se.seq <= last { continue; }
                                last = se.seq;
                            }
                            let frame = event_line(&sub_id2, &se);
                            if sub_tx.try_send(frame).is_err() {
                                // Full: drop oldest, count, emit Lagged.
                                let _ = sub_rx.try_recv();
                                dropped += 1;
                                let lag = event_line(&sub_id2, &SeqEvent {
                                    seq: 0,
                                    event: Event::Lagged { sub_id: sub_id2.clone(), dropped },
                                });
                                let _ = sub_tx.try_send(lag);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            // Never drop silently (F4): report the gap.
                            dropped += n;
                            let lag = event_line(&sub_id2, &SeqEvent {
                                seq: 0,
                                event: Event::Lagged { sub_id: sub_id2.clone(), dropped },
                            });
                            let _ = sub_tx.try_send(lag);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
    subs.insert(sub_id, cancel_tx, handle);
    Ok(result)
}

/// True when the event is a live-only, bus-only event (F12/F5.11/F6.5):
/// never journalled, so it carries no meaningful seq and must always be
/// forwarded rather than deduped against the replay cursor.
fn is_live_only(ev: &Event) -> bool {
    matches!(
        ev,
        Event::AssistantDelta { .. }
            | Event::ToolCall { .. }
            | Event::ToolResult { .. }
            | Event::ApprovalResolved { .. }
            | Event::WorkerDied { .. }
            | Event::WorkerRestarted { .. }
            | Event::ChannelMessage { .. }
            | Event::MemoryDegraded { .. }
    )
}

/// True when the event belongs to `run_id` (all variants except `Lagged`
/// carry a run id; `Lagged` is subscription-scoped and always forwarded).
fn event_matches_run(ev: &Event, run_id: &str) -> bool {
    match ev {
        Event::RunStarted { run_id: r, .. }
        | Event::NodeStarted { run_id: r, .. }
        | Event::NodeFinished { run_id: r, .. }
        | Event::NodeFailed { run_id: r, .. }
        | Event::NodeDead { run_id: r, .. }
        | Event::AssistantDelta { run_id: r, .. }
        | Event::ToolCall { run_id: r, .. }
        | Event::ToolResult { run_id: r, .. }
        | Event::RunFinished { run_id: r, .. }
        | Event::RunCancelled { run_id: r }
        | Event::RunSuspended { run_id: r, .. }
        | Event::RunResumed { run_id: r, .. }
        | Event::ApprovalRequested { run_id: r, .. } => r == run_id,
        // ApprovalResolved/Worker*/ChannelMessage/MemoryDegraded/Lagged are
        // process-scoped bus events, not per-run; forward them always.
        Event::ApprovalResolved { .. }
        | Event::WorkerDied { .. }
        | Event::WorkerRestarted { .. }
        | Event::ChannelMessage { .. }
        | Event::MemoryDegraded { .. }
        | Event::Lagged { .. } => true,
    }
}

/// True when the event's kind is in `kinds` (empty `kinds` = all).
fn event_matches_kinds(ev: &Event, kinds: &[String]) -> bool {
    if kinds.is_empty() {
        return true;
    }
    let kind = match ev {
        Event::RunStarted { .. } => "run_started",
        Event::NodeStarted { .. } => "node_started",
        Event::NodeFinished { .. } => "node_finished",
        Event::NodeFailed { .. } => "node_failed",
        Event::NodeDead { .. } => "node_dead",
        Event::AssistantDelta { .. } => "assistant_delta",
        Event::ToolCall { .. } => "tool_call",
        Event::ToolResult { .. } => "tool_result",
        Event::RunFinished { .. } => "run_finished",
        Event::RunCancelled { .. } => "run_cancelled",
        Event::RunSuspended { .. } => "run_suspended",
        Event::RunResumed { .. } => "run_resumed",
        Event::ApprovalRequested { .. } => "approval_requested",
        Event::ApprovalResolved { .. } => "approval_resolved",
        Event::WorkerDied { .. } => "worker_died",
        Event::WorkerRestarted { .. } => "worker_restarted",
        Event::ChannelMessage { .. } => "channel_message",
        Event::MemoryDegraded { .. } => "memory_degraded",
        Event::Lagged { .. } => "lagged",
    };
    kinds.iter().any(|k| k == kind)
}

fn event_line(sub_id: &str, se: &SeqEvent) -> String {
    let push = serde_json::json!({
        "sub_id": sub_id,
        "seq": se.seq,
        "event": se.event,
    });
    let note = Envelope::Notification(Notification {
        jsonrpc: "2.0".into(),
        method: Method::EventSubscribe,
        params: push,
    });
    write_frame(&note).unwrap_or_default()
}
