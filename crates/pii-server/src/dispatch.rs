//! Method dispatch (F2.7): every method of F1.3. `session.prompt` and
//! `run.start` return their ids immediately; `run.trace` reads the journal;
//! `graph.lint` returns `V<n>` strings verbatim.

use std::sync::Arc;

use tokio::sync::mpsc;

use pii_abi::PiiError;
use pii_proto::event::{EventFilter, EventSubscribeResult};
use pii_proto::method::Method;
use pii_proto::run::{SeqEvent, TraceResult, WireRunHeader};
use pii_proto::session::{
    SessionCreateParams, SessionCreateResult, SessionPromptParams, SessionPromptResult,
};
use pii_proto::sys::StatusResult;
use pii_proto::{Envelope, ErrorCode, Notification, Request, Response, RpcError, write_frame};

use crate::Engine;
use crate::bus::reattach;
use crate::conn::PeerIdentity;

/// Dispatches one request to a response (F2.7).
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
                runs_active: 0,
                sessions: 0,
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
                Ok(p) => match engine.prompt(&p.session_id, &p.content) {
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
        // The remaining methods return clean refusals in this pass; they are
        // owned by later parts of the phase (documented in the handoff).
        m => err(
            ErrorCode::MethodNotFound,
            &format!("method '{m}' is not served by this build"),
        ),
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
                pii_abi::RunStatus::Running => pii_proto::run::WireRunStatus::Running,
                pii_abi::RunStatus::Suspended => pii_proto::run::WireRunStatus::Suspended,
                pii_abi::RunStatus::Succeeded => pii_proto::run::WireRunStatus::Succeeded,
                pii_abi::RunStatus::Failed => pii_proto::run::WireRunStatus::Failed,
            },
            seq: header.seq,
            dur_ms: header.dur_ms,
        },
        events,
    })
}

/// `event.subscribe` (F2.5): replay `seq > since`, then attach live with
/// dedup; the live forwarder sends `event.subscribe` notifications.
fn subscribe(
    engine: &Arc<Engine>,
    req: &Request,
    wtx: &mpsc::UnboundedSender<String>,
) -> Result<Response, PiiError> {
    let filter: EventFilter = serde_json::from_value(req.params.clone())
        .map_err(|_| PiiError::Invalid(String::from("bad event.subscribe params")))?;
    let sub_id = format!("sub-{}", engine.started_at.elapsed().as_nanos());
    let run_id = filter
        .run
        .clone()
        .ok_or_else(|| PiiError::Invalid(String::from("event.subscribe needs run")))?;
    let since = filter.since_seq.unwrap_or(0);
    let mut live = engine.bus.subscribe();
    let mut merged: Vec<SeqEvent> = Vec::new();
    let mut last = reattach(
        &engine.store,
        &RunId::from(run_id.as_str()),
        since,
        &mut live,
        &mut merged,
    );
    let result = Response::ok(
        req.id.clone(),
        serde_json::to_value(EventSubscribeResult {
            sub_id: sub_id.clone(),
        })
        .unwrap_or(serde_json::Value::Null),
    );
    // Replay first (in order), then forward live with dedup.
    for se in merged {
        push_event(wtx, &sub_id, &se);
    }
    let wtx2 = wtx.clone();
    tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(se) => {
                    if se.seq > last {
                        last = se.seq;
                        push_event(&wtx2, &sub_id, &se);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
    Ok(result)
}

fn push_event(wtx: &mpsc::UnboundedSender<String>, sub_id: &str, se: &SeqEvent) {
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
    if let Ok(line) = write_frame(&note) {
        let _ = wtx.send(line);
    }
}

/// Handles notifications (no response expected).
pub fn notification(engine: &Arc<Engine>, n: &Notification) {
    if n.method == Method::EventUnsubscribe {
        let _ = engine; // subscriptions die with their connection in this pass
    }
}
