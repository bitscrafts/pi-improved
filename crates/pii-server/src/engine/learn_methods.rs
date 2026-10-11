//! The daemon's `learn.*` writes (spec 138).
//!
//! The daemon is the only writer of `learn/`: it applies a promotion and
//! records it in one step, under the writer lock in `pii_learn::Promoter`.
//! The CLI asks over the control socket and never writes the ledger itself.

use std::path::Path;
use std::sync::Arc;

use pii_learn::{PromoteConfig, PromoteRequest, Promoter, ledger_id, ledger_tail};
use pii_proto::{ErrorCode, Request, Response, RpcError};

use crate::Engine;
use crate::engine::runs::learn_now_secs;

fn err(req: &Request, c: ErrorCode, m: &str) -> Response {
    Response::err(req.id.clone(), RpcError::new(c, m))
}

/// The daemon's own store dir, after checking the caller means the same one:
/// a CLI pointed at another `--dir` must not be silently written to this one.
fn store_dir(engine: &Arc<Engine>, req: &Request) -> Result<std::path::PathBuf, Box<Response>> {
    let Some(ctx) = engine.daemon_ctx() else {
        return Err(Box::new(err(req, ErrorCode::NotFound, "no daemon context")));
    };
    let own = ctx.runtime_dir.clone();
    if let Some(asked) = req.params.get("dir").and_then(serde_json::Value::as_str) {
        let same = |a: &Path, b: &Path| match (dunce::canonicalize(a), dunce::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => a == b,
        };
        if !same(Path::new(asked), &own) {
            return Err(Box::new(err(
                req,
                ErrorCode::InvalidParams,
                &format!(
                    "the daemon owns the ledger under {}, not {asked}",
                    own.display()
                ),
            )));
        }
    }
    Ok(own)
}

/// The hash of `name`'s definition on disk (spec 139): the canonical JSON of the
/// parsed graph, so a whitespace or comment edit is not a change; the raw bytes
/// when it does not parse. `None` when there is no such graph file.
fn definition_hash(graph_dir: &Path, name: &str) -> Option<String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    let src = std::fs::read_to_string(graph_dir.join(format!("{name}.toml"))).ok()?;
    let body = pii_graph::toml_load::load_toml(&src)
        .and_then(|s| s.to_json())
        .unwrap_or_else(|_| src.into_bytes());
    Some(blake3::hash(&body).to_hex().to_string())
}

fn promoter(engine: &Engine, dir: &Path) -> Promoter {
    let graph_dir = engine.cfg.graph_dir.clone();
    Promoter::new(dir, PromoteConfig::default(), engine.log())
        .with_definitions(Arc::new(move |g: &str| definition_hash(&graph_dir, g)))
}

fn str_param(req: &Request, key: &str) -> Option<String> {
    req.params
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn u64_param(req: &Request, key: &str) -> Option<u64> {
    req.params.get(key).and_then(serde_json::Value::as_u64)
}

/// `learn.promote`: `{candidate, kind, actor, metric?, window_secs?, ttl_secs?, dir?}`.
/// A refusal is a normal answer (`{"refused", "ledger"}`), not a transport error.
pub(crate) fn learn_promote(engine: &Arc<Engine>, req: &Request) -> Response {
    let dir = match store_dir(engine, req) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let (Some(candidate), Some(actor)) = (str_param(req, "candidate"), str_param(req, "actor"))
    else {
        return err(
            req,
            ErrorCode::InvalidParams,
            "learn.promote needs candidate and actor",
        );
    };
    let r = PromoteRequest {
        candidate,
        kind: str_param(req, "kind").unwrap_or_else(|| "quarantine_graph".to_owned()),
        actor,
        metric: str_param(req, "metric"),
        window_secs: u64_param(req, "window_secs"),
        ttl_secs: u64_param(req, "ttl_secs"),
    };
    let body = match promoter(engine, &dir).promote(&r, learn_now_secs()) {
        Ok(p) => {
            let ledger = ledger_tail(&dir, 8)
                .iter()
                .rev()
                .find(|e| e.promotion == p.id && e.event == "applied")
                .map(|e| ledger_id(e.seq));
            serde_json::json!({"promotion": p, "ledger": ledger})
        }
        Err(r) => serde_json::json!({"refused": r.to_string(), "ledger": r.ledger}),
    };
    Response::ok(req.id.clone(), body)
}

/// `learn.revert`: `{id, actor, dir?}` → `{"reverted", "ledger"}`.
pub(crate) fn learn_revert(engine: &Arc<Engine>, req: &Request) -> Response {
    let dir = match store_dir(engine, req) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let (Some(id), Some(actor)) = (str_param(req, "id"), str_param(req, "actor")) else {
        return err(
            req,
            ErrorCode::InvalidParams,
            "learn.revert needs id and actor",
        );
    };
    match promoter(engine, &dir).revert(&id, &actor, learn_now_secs()) {
        Ok(l) => Response::ok(
            req.id.clone(),
            serde_json::json!({"reverted": id, "ledger": l}),
        ),
        Err(e) => err(req, ErrorCode::InvalidParams, &e),
    }
}

/// `learn.sweep`: `{dir?}` → `{"changed": [[id, state], ...]}`.
pub(crate) fn learn_sweep(engine: &Arc<Engine>, req: &Request) -> Response {
    let dir = match store_dir(engine, req) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let changed: Vec<(String, &str)> = promotion_sweep(engine, &dir)
        .into_iter()
        .map(|(id, st)| (id, st.as_str()))
        .collect();
    Response::ok(req.id.clone(), serde_json::json!({"changed": changed}))
}

/// One lifecycle pass over the promotions under `dir`, now.
pub(crate) fn promotion_sweep(
    engine: &Engine,
    dir: &Path,
) -> Vec<(String, pii_learn::PromotionState)> {
    promotion_sweep_at(engine, dir, learn_now_secs())
}

/// One lifecycle pass at `now`: lapses what is past its TTL and evaluates what
/// has closed its window against the graphs' current definitions.
pub(crate) fn promotion_sweep_at(
    engine: &Engine,
    dir: &Path,
    now: u64,
) -> Vec<(String, pii_learn::PromotionState)> {
    promoter(engine, dir).sweep(now)
}

/// One unattended extraction + verification pass (spec 139 R1): what the daemon's
/// learn timer runs. It proposes and verifies; it has no path to a promotion.
///
/// # Errors
/// A message when no trajectory root is attached or the pass cannot complete.
pub(crate) fn learn_unattended_pass(
    engine: &Engine,
    dir: &Path,
) -> Result<pii_learn::UnattendedStatus, String> {
    let root = engine
        .learn_traj_root()
        .ok_or_else(|| String::from("no trajectory root attached ([core].trajectory_dir)"))?;
    let mut verify = pii_learn::VerifyConfig::new(root.clone(), engine.cfg.graph_dir.clone());
    verify.scratch_dir = dir.join("scratch");
    verify.protected.push(dir.to_path_buf());
    let cfg = pii_learn::UnattendedConfig {
        learn: pii_learn::LearnConfig::default(),
        verify,
    };
    pii_learn::run_unattended(&root, dir, &cfg, &engine.log(), learn_now_secs())
}
