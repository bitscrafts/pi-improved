//! Protocol handlers for the daemon-only methods (F7.1).
//!
//! `worker.*`, `approve.*`, `cron.*`, `channel.*`, `memory.*`, `sys.reload`,
//! `run.resume`. Split out of [`crate::dispatch`] to keep that file under
//! the 450-line gate.

use std::sync::Arc;

use pii_abi::RunId;
use pii_core::Clock as _;
use pii_memory::consolidate::{ConsolidateCfg, consolidate};
use pii_memory::doc::write_fact;
use pii_memory::embed::TrigramEmbedder;
use pii_memory::search::MemorySearcher;
use pii_proto::sys::StatusResult;
use pii_proto::{ErrorCode, Request, Response, RpcError};

use crate::Engine;
use crate::dispatch::{err_code, value_response};

fn err(req: &Request, c: ErrorCode, m: &str) -> Response {
    Response::err(req.id.clone(), RpcError::new(c, m))
}

/// `sys.status` (F5.12): rss, protocol, and the daemon subsystems.
#[must_use]
pub(crate) fn status_result(engine: &Arc<Engine>) -> StatusResult {
    let uptime_ms = u64::try_from(engine.started_at.elapsed().as_millis()).unwrap_or(0);
    let runs_active = u32::try_from(engine.runs.lock().len()).unwrap_or(u32::MAX);
    let sessions = u32::try_from(engine.sessions.lock().len()).unwrap_or(u32::MAX);
    let sandbox = pii_tools::sandbox::profile_name(engine.cfg.sandbox);
    let docs = engine.memory.n_docs().unwrap_or(0);
    engine.daemon_ctx().map_or_else(
        || StatusResult {
            uptime_ms,
            runs_active,
            sessions,
            sandbox_profile: sandbox.to_owned(),
            protocol: pii_proto::PROTOCOL_VERSION.to_owned(),
            rss_kb: crate::daemon::status::rss_kb(),
            workers: Vec::new(),
            listeners: Vec::new(),
            channels: Vec::new(),
            memory: pii_proto::sys::MemoryStatus { docs },
        },
        |ctx| ctx.status(uptime_ms, runs_active, sessions, sandbox, docs),
    )
}

/// `worker.list`.
pub(crate) fn worker_list(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(ctx) = engine.daemon_ctx() else {
        return value_response(req, Some(serde_json::json!([])), |c, m| err(req, c, m));
    };
    let list: Vec<serde_json::Value> = ctx
        .workers
        .lock()
        .values()
        .map(|s| {
            let i = s.info();
            serde_json::json!({
                "name": i.name, "state": format!("{:?}", i.state),
                "pid": i.pid, "restarts": i.restarts, "in_flight": i.in_flight,
            })
        })
        .collect();
    value_response(req, Some(serde_json::json!(list)), |c, m| err(req, c, m))
}

/// `worker.reload { name }` (F3.6).
pub(crate) fn worker_reload(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(name) = req.params.get("name").and_then(|v| v.as_str()) else {
        return err(req, ErrorCode::InvalidParams, "worker.reload needs 'name'");
    };
    let Some(ctx) = engine.daemon_ctx() else {
        return err(req, ErrorCode::NotFound, "no daemon context");
    };
    let Some(sup) = ctx.workers.lock().get(name).cloned() else {
        return err(req, ErrorCode::NotFound, "unknown worker");
    };
    let mut caps = pii_core::CapSet::empty();
    caps.kinds = pii_core::CapKind::PROC | pii_core::CapKind::SPAWN;
    caps.proc_argv0 = pii_core::StringScope::new(vec![String::new()]);
    let rctx = pii_core::Ctx::root(caps, pii_core::Budget::unlimited());
    match sup.reload(&rctx) {
        Ok(r) => value_response(
            req,
            Some(serde_json::json!({
                "old_pid": r.old_pid, "new_pid": r.new_pid, "drained": r.drained,
            })),
            |c, m| err(req, c, m),
        ),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `approve.list`.
pub(crate) fn approve_list(engine: &Arc<Engine>, req: &Request) -> Response {
    match engine.approvals.pending_approvals() {
        Ok(list) => {
            let v: Vec<serde_json::Value> = list
                .into_iter()
                .map(|a| {
                    serde_json::json!({
                        "approval_id": a.approval_id, "run_id": a.run_id, "node": a.node,
                        "policy": a.policy, "prompt": a.prompt,
                        "requested_at_ms": a.requested_at_ms,
                    })
                })
                .collect();
            value_response(req, Some(serde_json::json!(v)), |c, m| err(req, c, m))
        }
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `approve.resolve { id, allow, note? }` (F5.10).
pub(crate) fn approve_resolve(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(id) = req.params.get("id").and_then(|v| v.as_str()) else {
        return err(req, ErrorCode::InvalidParams, "approve.resolve needs 'id'");
    };
    let Some(allow) = req.params.get("allow").and_then(serde_json::Value::as_bool) else {
        return err(
            req,
            ErrorCode::InvalidParams,
            "approve.resolve needs 'allow'",
        );
    };
    let note = req.params.get("note").and_then(|v| v.as_str());
    match crate::daemon::approvals::resolve(engine, id, allow, note) {
        Ok(outcome) => value_response(
            req,
            Some(serde_json::json!({ "resumed": outcome.resumed })),
            |c, m| err(req, c, m),
        ),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `heal.list` (S0.4): every heal-attempt ledger entry.
pub(crate) fn heal_list(engine: &Arc<Engine>, req: &Request) -> Response {
    match engine.heals.list_attempts() {
        Ok(list) => {
            let v: Vec<serde_json::Value> = list
                .into_iter()
                .filter_map(|a| serde_json::to_value(&a).ok())
                .collect();
            value_response(req, Some(serde_json::json!(v)), |c, m| err(req, c, m))
        }
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `heal.get { signature_hash }` (S0.4): one entry's full attempt history.
pub(crate) fn heal_get(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(sig) = req.params.get("signature_hash").and_then(|v| v.as_str()) else {
        return err(
            req,
            ErrorCode::InvalidParams,
            "heal.get needs 'signature_hash'",
        );
    };
    match engine.heals.get_attempt(sig) {
        Ok(Some(a)) => value_response(req, serde_json::to_value(&a).ok(), |c, m| err(req, c, m)),
        Ok(None) => err(
            req,
            ErrorCode::NotFound,
            "no heal attempt with that signature",
        ),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `cron.list`.
pub(crate) fn cron_list(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(ctx) = engine.daemon_ctx() else {
        return value_response(req, Some(serde_json::json!([])), |c, m| err(req, c, m));
    };
    let list: Vec<serde_json::Value> = ctx
        .cron
        .lock()
        .values()
        .map(|c| serde_json::json!({ "name": c.name, "graph": c.graph, "schedule": c.schedule }))
        .collect();
    value_response(req, Some(serde_json::json!(list)), |c, m| err(req, c, m))
}

/// `channel.list` (F6.8: never carries the token).
pub(crate) fn channel_list(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(ctx) = engine.daemon_ctx() else {
        return value_response(req, Some(serde_json::json!([])), |c, m| err(req, c, m));
    };
    let status = ctx.status(0, 0, 0, "", 0);
    value_response(req, serde_json::to_value(status.channels).ok(), |c, m| {
        err(req, c, m)
    })
}

/// `memory.search { q, k? }`.
pub(crate) fn memory_search(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(q) = req.params.get("q").and_then(|v| v.as_str()) else {
        return err(req, ErrorCode::InvalidParams, "memory.search needs 'q'");
    };
    let k = usize::try_from(
        req.params
            .get("k")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(6),
    )
    .unwrap_or(6);
    let searcher = MemorySearcher::new(Arc::clone(&engine.memory), Some(Arc::new(TrigramEmbedder)));
    let rctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
    match searcher.search(&rctx, q, k) {
        Ok(r) => {
            let hits: Vec<serde_json::Value> = r
                .hits
                .iter()
                .map(|h| serde_json::json!({"doc": h.doc, "score": h.score, "lanes": h.lanes}))
                .collect();
            value_response(
                req,
                Some(serde_json::json!({"hits": hits, "degraded": r.degraded, "lanes": r.lanes})),
                |c, m| err(req, c, m),
            )
        }
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `memory.put { layer, kind, text, concepts?, importance? }`.
pub(crate) fn memory_put(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(text) = req.params.get("text").and_then(|v| v.as_str()) else {
        return err(req, ErrorCode::InvalidParams, "memory.put needs 'text'");
    };
    let concepts: Vec<String> = req
        .params
        .get("concepts")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    #[allow(clippy::cast_possible_truncation)] // reason: importance is a
    // 0..~10 weight; f64->f32 precision loss here is immaterial.
    let importance = req
        .params
        .get("importance")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0) as f32;
    let now_ms = pii_core::SystemClock.now_ms();
    // G2.5/G2.8 (spec 09 v2): use the shared `write_fact` helper so the
    // vector lane gets real data (the old inline path wrote `vector: None`).
    let rows = write_fact(text, concepts, importance, now_ms);
    match engine.memory.put(&rows) {
        Ok(()) => value_response(req, Some(serde_json::json!({"id": rows.id})), |c, m| {
            err(req, c, m)
        }),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `memory.consolidate` (F4.12: zero model calls).
pub(crate) fn memory_consolidate(engine: &Arc<Engine>, req: &Request) -> Response {
    let rctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
    let cfg = ConsolidateCfg {
        archive_after_days: 90.0,
        same_layer_only: true,
    };
    match consolidate(&engine.memory, &rctx, &cfg) {
        Ok(r) => value_response(
            req,
            Some(serde_json::json!({
                "candidates_evaluated": r.candidates_evaluated,
                "candidates_capped": r.candidates_capped,
                "merged": r.merged, "archived": r.archived,
            })),
            |c, m| err(req, c, m),
        ),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `sys.reload` (F5.12): re-reads the config file, applies it to channels
/// and workers, never restarts the process.
pub(crate) fn sys_reload(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(ctx) = engine.daemon_ctx() else {
        return err(req, ErrorCode::NotFound, "no daemon context");
    };
    match crate::daemon::reload_config(&ctx) {
        Ok(n) => value_response(
            req,
            Some(serde_json::json!({"workers_reloaded": n})),
            |c, m| err(req, c, m),
        ),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `run.resume { run_id }`: manual resume (approvals/timers resume
/// themselves; this is the operator escape hatch).
pub(crate) fn run_resume(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(run_id) = req.params.get("run_id").and_then(|v| v.as_str()) else {
        return err(req, ErrorCode::InvalidParams, "run.resume needs 'run_id'");
    };
    match crate::runs::spawn_resume(Arc::clone(engine), RunId::from(run_id), None) {
        Ok(()) => value_response(req, Some(serde_json::json!({"resumed": true})), |c, m| {
            err(req, c, m)
        }),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}
