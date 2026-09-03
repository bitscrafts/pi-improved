//! Method dispatch (F2.7): every method of F1.3. `session.prompt` and
//! `run.start` return their ids immediately; `run.trace` reads the journal;
//! `graph.lint` returns `V<n>` strings verbatim.

use std::sync::Arc;

use tokio::sync::mpsc;

use pii_abi::PiiError;
use pii_proto::event::{Event, EventFilter, EventSubscribeResult};
use pii_proto::method::Method;
use pii_proto::run::{
    GraphLintParams, GraphLintResult, RunListParams, RunStartParams, RunStartResult, SeqEvent,
    TraceResult, WireRunHeader, WireRunStatus,
};
use pii_proto::session::{
    SessionCreateParams, SessionCreateResult, SessionPromptParams, SessionPromptResult,
};
use pii_proto::sys::StatusResult;
use pii_proto::{Envelope, ErrorCode, Notification, Request, Response, RpcError, write_frame};

use crate::Engine;
use crate::bus::reattach;
use crate::conn::PeerIdentity;

/// Dispatches one request to a response (F2.7).
// reason: a method-match dispatch is the natural form; extracting every arm
// into a helper would obscure the routing table.
#[allow(clippy::too_many_lines)]
pub fn dispatch(
    engine: &Arc<Engine>,
    req: &Request,
    peer: PeerIdentity,
    wtx: &mpsc::UnboundedSender<String>,
) -> Response {
    let _ = peer;
    let err = |c: ErrorCode, m: &str| Response::err(req.id.clone(), RpcError::new(c, m));
    match req.method {
        Method::SysHello => err(ErrorCode::InvalidRequest, "hello already handled"),
        Method::SysStatus => {
            let status = StatusResult {
                uptime_ms: u64::try_from(engine.started_at.elapsed().as_millis()).unwrap_or(0),
                runs_active: u32::try_from(engine.runs.lock().len()).unwrap_or(u32::MAX),
                sessions: u32::try_from(engine.sessions.lock().len()).unwrap_or(u32::MAX),
                sandbox_profile: pii_tools::sandbox::profile_name(engine.cfg.sandbox).to_owned(),
            };
            value_response(req, serde_json::to_value(status).ok(), err)
        }
        Method::SysVersion => value_response(
            req,
            Some(serde_json::json!({"version": env!("CARGO_PKG_VERSION")})),
            err,
        ),
        Method::SysShutdown => Response::ok(req.id.clone(), serde_json::json!({"bye": true})),
        Method::SessionCreate => {
            match serde_json::from_value::<SessionCreateParams>(req.params.clone()) {
                Ok(p) => {
                    let sid = engine.create_session(p.cwd, p.tools, p.graph);
                    value_response(
                        req,
                        serde_json::to_value(SessionCreateResult { session_id: sid }).ok(),
                        err,
                    )
                }
                Err(_) => err(ErrorCode::InvalidParams, "bad session.create params"),
            }
        }
        Method::SessionPrompt => {
            match serde_json::from_value::<SessionPromptParams>(req.params.clone()) {
                Ok(p) => match engine.prompt(&p.session_id, &p.content, p.input) {
                    Ok(run_id) => value_response(
                        req,
                        serde_json::to_value(SessionPromptResult {
                            run_id: run_id.to_string(),
                        })
                        .ok(),
                        err,
                    ),
                    Err(e) => err_code(err, &e),
                },
                Err(_) => err(ErrorCode::InvalidParams, "bad session.prompt params"),
            }
        }
        Method::SessionState => {
            let sid = req.params.get("session_id").and_then(|v| v.as_str());
            sid.and_then(|s| engine.session_state(s)).map_or_else(
                || err(ErrorCode::NotFound, "unknown session"),
                |state| value_response(req, serde_json::to_value(state).ok(), err),
            )
        }
        Method::RunCancel => {
            if let Some(rid) = req.params.get("run_id").and_then(|v| v.as_str()) {
                engine.cancel(rid);
            }
            Response::ok(req.id.clone(), serde_json::json!({"cancelled": true}))
        }
        Method::RunTrace => {
            let rid = req.params.get("run_id").and_then(|v| v.as_str());
            match rid.map(RunId::from).map(|r| trace(engine, &r)) {
                Some(Ok(t)) => value_response(req, serde_json::to_value(t).ok(), err),
                Some(Err(e)) => err_code(err, &e),
                None => err(ErrorCode::InvalidParams, "run.trace needs run_id"),
            }
        }
        Method::EventSubscribe => subscribe(engine, req, wtx).unwrap_or_else(|e| err_code(err, &e)),
        Method::RunStart => run_start(engine, req, &err),
        Method::RunGet => run_get(engine, req, &err),
        Method::RunList => run_list(engine, req, &err),
        Method::GraphLint => graph_lint_dispatch(engine, req, &err),
        Method::EventUnsubscribe => {
            // F6: subscriptions die with their connection in this pass; a
            // no-op ack keeps the client from hanging.
            Response::ok(req.id.clone(), serde_json::json!({"unsubscribed": true}))
        }
        // The remaining methods return clean refusals in this pass; they are
        // owned by later parts of the phase (documented in the handoff).
        m => err(
            ErrorCode::MethodNotFound,
            &format!("method '{m}' is not served by this build"),
        ),
    }
}

/// `run.start` (F6): start a graph run by name, return its id immediately.
fn run_start(
    engine: &Arc<Engine>,
    req: &Request,
    err: &impl Fn(ErrorCode, &str) -> Response,
) -> Response {
    match serde_json::from_value::<RunStartParams>(req.params.clone()) {
        Ok(p) => match engine.start_run(&p.graph, &p.input) {
            Ok(run_id) => value_response(
                req,
                serde_json::to_value(RunStartResult {
                    run_id: run_id.to_string(),
                })
                .ok(),
                err,
            ),
            Err(e) => err_code(err, &e),
        },
        Err(_) => err(ErrorCode::InvalidParams, "bad run.start params"),
    }
}

/// `run.get` (F6): fetch a run header.
fn run_get(
    engine: &Arc<Engine>,
    req: &Request,
    err: &impl Fn(ErrorCode, &str) -> Response,
) -> Response {
    let rid = req.params.get("run_id").and_then(|v| v.as_str());
    match rid.map(RunId::from).map(|r| run_header(engine, &r)) {
        Some(Ok(h)) => value_response(req, serde_json::to_value(h).ok(), err),
        Some(Err(e)) => err_code(err, &e),
        None => err(ErrorCode::InvalidParams, "run.get needs run_id"),
    }
}

/// `run.list` (F6): list run headers, optionally filtered by status.
fn run_list(
    engine: &Arc<Engine>,
    req: &Request,
    err: &impl Fn(ErrorCode, &str) -> Response,
) -> Response {
    let params: RunListParams =
        serde_json::from_value(req.params.clone()).unwrap_or(RunListParams {
            status: None,
            since: None,
        });
    let status = params.status.map(|s| match s {
        WireRunStatus::Running => pii_abi::RunStatus::Running,
        WireRunStatus::Suspended => pii_abi::RunStatus::Suspended,
        WireRunStatus::Succeeded => pii_abi::RunStatus::Succeeded,
        WireRunStatus::Failed => pii_abi::RunStatus::Failed,
    });
    let runs = status.map_or_else(Vec::new, |st| {
        engine.store.runs_by_status(st).unwrap_or_default()
    });
    let headers: Vec<WireRunHeader> = runs
        .iter()
        .filter_map(|r| run_header(engine, r).ok())
        .collect();
    value_response(req, serde_json::to_value(headers).ok(), err)
}

/// `graph.lint` (F6): lint a TOML graph source.
fn graph_lint_dispatch(
    engine: &Arc<Engine>,
    req: &Request,
    err: &impl Fn(ErrorCode, &str) -> Response,
) -> Response {
    match serde_json::from_value::<GraphLintParams>(req.params.clone()) {
        Ok(p) => {
            let result = graph_lint(engine, &p.toml);
            value_response(req, serde_json::to_value(result).ok(), err)
        }
        Err(_) => err(ErrorCode::InvalidParams, "bad graph.lint params"),
    }
}

/// Reads a run header as a wire header (F6: `run.get`/`run.list`).
fn run_header(engine: &Arc<Engine>, run_id: &RunId) -> Result<WireRunHeader, PiiError> {
    let header = engine
        .store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}'")))?;
    Ok(WireRunHeader {
        run_id: header.run.to_string(),
        graph: header.graph.clone(),
        status: match header.status {
            pii_abi::RunStatus::Running => WireRunStatus::Running,
            pii_abi::RunStatus::Suspended => WireRunStatus::Suspended,
            pii_abi::RunStatus::Succeeded => WireRunStatus::Succeeded,
            pii_abi::RunStatus::Failed => WireRunStatus::Failed,
        },
        seq: header.seq,
        dur_ms: header.dur_ms,
    })
}

/// Lints a TOML graph source (F6: `graph.lint`).
fn graph_lint(engine: &Arc<Engine>, toml: &str) -> GraphLintResult {
    match pii_graph::load_toml(toml) {
        Ok(spec) => {
            let reg = crate::lint_registry(&engine.cfg.workspace);
            match pii_graph::validate(&spec, &reg) {
                Ok(warnings) => GraphLintResult {
                    ok: true,
                    errors: Vec::new(),
                    warnings,
                },
                Err(errors) => GraphLintResult {
                    ok: false,
                    errors,
                    warnings: Vec::new(),
                },
            }
        }
        Err(e) => GraphLintResult {
            ok: false,
            errors: vec![e.to_string()],
            warnings: Vec::new(),
        },
    }
}

use pii_abi::RunId;

fn err_code(err: impl Fn(ErrorCode, &str) -> Response, e: &PiiError) -> Response {
    let code = match e {
        PiiError::NotFound(_) => ErrorCode::NotFound,
        PiiError::Denied(_) => ErrorCode::Denied,
        PiiError::Cancelled => ErrorCode::Cancelled,
        PiiError::BudgetExhausted(_) => ErrorCode::BudgetBreach,
        _ => ErrorCode::Internal,
    };
    err(code, &e.to_string())
}

fn value_response(
    req: &Request,
    value: Option<serde_json::Value>,
    err: impl Fn(ErrorCode, &str) -> Response,
) -> Response {
    value.map_or_else(
        || err(ErrorCode::Internal, "result serialisation failed"),
        |v| Response::ok(req.id.clone(), v),
    )
}

fn trace(engine: &Arc<Engine>, run_id: &RunId) -> Result<TraceResult, PiiError> {
    let header = engine
        .store
        .get_run(run_id)?
        .ok_or_else(|| PiiError::NotFound(format!("run '{run_id}'")))?;
    let journal = engine.store.read_journal(run_id)?;
    let events = journal
        .into_iter()
        .filter_map(|(seq, ev)| {
            crate::bus::map_event(run_id, &ev).map(|event| SeqEvent { seq, event })
        })
        .collect();
    Ok(TraceResult {
        header: WireRunHeader {
            run_id: header.run.to_string(),
            graph: header.graph.clone(),
            status: match header.status {
                pii_abi::RunStatus::Running => WireRunStatus::Running,
                pii_abi::RunStatus::Suspended => WireRunStatus::Suspended,
                pii_abi::RunStatus::Succeeded => WireRunStatus::Succeeded,
                pii_abi::RunStatus::Failed => WireRunStatus::Failed,
            },
            seq: header.seq,
            dur_ms: header.dur_ms,
        },
        events,
    })
}

/// `event.subscribe` (F2.5): replay `seq > since`, then attach live with
/// dedup. The live forwarder filters by the subscription's run and kinds
/// (F3) and uses a bounded per-subscription queue that emits `Lagged`
/// instead of dropping silently (F4).
// reason: the forwarder is a single select! loop; splitting it would obscure
// the replay/live/dedup/lag handling.
#[allow(clippy::too_many_lines)]
fn subscribe(
    engine: &Arc<Engine>,
    req: &Request,
    wtx: &mpsc::UnboundedSender<String>,
) -> Result<Response, PiiError> {
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
    let sub_id2 = sub_id;
    tokio::spawn(async move {
        let mut dropped: u64 = 0;
        loop {
            tokio::select! {
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
    Ok(result)
}

/// True when the event is a live-only, bus-only event (F12): never journalled,
/// so it carries no meaningful seq and must always be forwarded.
fn is_live_only(ev: &Event) -> bool {
    matches!(
        ev,
        Event::AssistantDelta { .. } | Event::ToolCall { .. } | Event::ToolResult { .. }
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
        | Event::RunCancelled { run_id: r } => r == run_id,
        Event::Lagged { .. } => true,
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

/// Handles notifications (no response expected).
pub fn notification(engine: &Arc<Engine>, n: &Notification) {
    if n.method == Method::EventUnsubscribe {
        let _ = engine; // subscriptions die with their connection in this pass
    }
}
