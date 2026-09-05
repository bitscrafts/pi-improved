//! Method dispatch (F2.7): every method of F1.3. `session.prompt` and
//! `run.start` return their ids immediately; `run.trace` reads the journal;
//! `graph.lint` returns `V<n>` strings verbatim.
//!
//! `event.subscribe`/`event.unsubscribe` live in [`crate::subscribe`] (M1,
//! opus review — split out to stay under the 450-line lint cap after real
//! subscription cancellation replaced the old ack-only no-op).

use std::sync::Arc;

use tokio::sync::mpsc;

use pii_abi::PiiError;
use pii_proto::method::Method;
use pii_proto::run::{
    GraphLintParams, GraphLintResult, RunListParams, RunStartParams, RunStartResult, SeqEvent,
    TraceResult, WireRunHeader, WireRunStatus,
};
use pii_proto::session::{
    SessionCreateParams, SessionCreateResult, SessionPromptParams, SessionPromptResult,
};
use pii_proto::{ErrorCode, Notification, Request, Response, RpcError};

use crate::Engine;
use crate::conn::{PeerIdentity, SubscriptionRegistry};
use crate::subscribe;

/// Dispatches one request to a response (F2.7).
// reason: a method-match dispatch is the natural form; extracting every arm
// into a helper would obscure the routing table.
#[allow(clippy::too_many_lines)]
pub fn dispatch(
    engine: &Arc<Engine>,
    req: &Request,
    peer: PeerIdentity,
    wtx: &mpsc::UnboundedSender<String>,
    subs: &SubscriptionRegistry,
) -> Response {
    let _ = peer;
    let err = |c: ErrorCode, m: &str| Response::err(req.id.clone(), RpcError::new(c, m));
    match req.method {
        Method::SysHello => err(ErrorCode::InvalidRequest, "hello already handled"),
        Method::SysStatus => value_response(
            req,
            serde_json::to_value(crate::daemon::methods::status_result(engine)).ok(),
            err,
        ),
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
        Method::EventSubscribe => {
            subscribe::subscribe(engine, req, wtx, subs).unwrap_or_else(|e| err_code(err, &e))
        }
        Method::RunStart => run_start(engine, req, &err),
        Method::RunGet => run_get(engine, req, &err),
        Method::RunList => run_list(engine, req, &err),
        Method::GraphLint => graph_lint_dispatch(engine, req, &err),
        Method::EventUnsubscribe => {
            // M1 (opus review): actually cancel the subscriber task instead
            // of an ack-only no-op — the previous behaviour left the task +
            // its bounded queue running until the whole connection dropped.
            let sub_id = req
                .params
                .get("sub_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let cancelled = subs.cancel(sub_id);
            Response::ok(
                req.id.clone(),
                serde_json::json!({"unsubscribed": cancelled}),
            )
        }
        Method::WorkerList => crate::daemon::methods::worker_list(engine, req),
        Method::WorkerReload => crate::daemon::methods::worker_reload(engine, req),
        Method::ApproveList => crate::daemon::methods::approve_list(engine, req),
        Method::ApproveResolve => crate::daemon::methods::approve_resolve(engine, req),
        Method::HealList => crate::daemon::methods::heal_list(engine, req),
        Method::HealGet => crate::daemon::methods::heal_get(engine, req),
        Method::CronList => crate::daemon::methods::cron_list(engine, req),
        Method::ChannelList => crate::daemon::methods::channel_list(engine, req),
        Method::MemorySearch => crate::daemon::methods::memory_search(engine, req),
        Method::MemoryPut => crate::daemon::methods::memory_put(engine, req),
        Method::MemoryConsolidate => crate::daemon::methods::memory_consolidate(engine, req),
        Method::SysReload => crate::daemon::methods::sys_reload(engine, req),
        Method::RunResume => crate::daemon::methods::run_resume(engine, req),
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
    // B3 (opus review): derive the lint scratch dir from this engine's own
    // configured store path instead of a hardcoded absolute string, so
    // lint works the same on any machine/runtime dir.
    let runtime_dir = engine.cfg.store_path.parent().map_or_else(
        || std::path::PathBuf::from("."),
        std::path::Path::to_path_buf,
    );
    let reg = crate::lint_registry(&engine.cfg.workspace, &runtime_dir);
    match pii_graph::load_toml_with_registry(toml, &reg) {
        Ok(spec) => match pii_graph::validate(&spec, &reg) {
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
        },
        Err(e) => GraphLintResult {
            ok: false,
            errors: vec![e.to_string()],
            warnings: Vec::new(),
        },
    }
}

use pii_abi::RunId;

pub(crate) fn err_code(err: impl Fn(ErrorCode, &str) -> Response, e: &PiiError) -> Response {
    let code = match e {
        PiiError::NotFound(_) => ErrorCode::NotFound,
        PiiError::Denied(_) => ErrorCode::Denied,
        PiiError::Cancelled => ErrorCode::Cancelled,
        PiiError::BudgetExhausted(_) => ErrorCode::BudgetBreach,
        _ => ErrorCode::Internal,
    };
    err(code, &e.to_string())
}

pub(crate) fn value_response(
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

/// Handles notifications (no response expected).
///
/// `event.unsubscribe` (M1, opus review) is dispatched as a `Request` via
/// [`dispatch`] → [`crate::subscribe::subscribe`]'s cancellation path, not
/// through this notification handler — a `Notification` has no id to
/// acknowledge on, so a client that sends it as one gets no response and no
/// cancellation. This is intentionally a no-op, not a leak.
pub fn notification(engine: &Arc<Engine>, n: &Notification) {
    let _ = (engine, n);
}
