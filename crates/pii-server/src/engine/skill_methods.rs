//! `skill.put` and `skill.list` (spec 233 R1, R2, R4).
//!
//! The daemon writes and reads its own memory store, the one the `coder` node's `StoreSkills` reads, so a skill put
//! here is pinnable by the very next run, with no restart.

use std::sync::Arc;

use pii_core::Clock as _;
use pii_memory::doc::{Layer, layer_of_byte};
use pii_memory::skill::{AddError, add, listing};
use pii_proto::{ErrorCode, Request, Response, RpcError};

use crate::Engine;
use crate::engine::dispatch::{err_code, value_response};

/// `skill.put {text, layer?}`: stores the text as one skill document (default layer L3). Answers `{id, slug, sha256}`.
/// A text with no marker, or a slug a live skill holds, is refused and writes nothing.
pub(crate) fn skill_put(engine: &Arc<Engine>, req: &Request) -> Response {
    let err = |c: ErrorCode, m: &str| Response::err(req.id.clone(), RpcError::new(c, m));
    let Some(text) = req.params.get("text").and_then(serde_json::Value::as_str) else {
        return err(ErrorCode::InvalidParams, "skill.put needs 'text'");
    };
    let tag = req
        .params
        .get("layer")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(2);
    if tag > 2 {
        return err(
            ErrorCode::InvalidParams,
            "skill.put layer must be 0, 1 or 2",
        );
    }
    let layer: Layer = layer_of_byte(u8::try_from(tag).unwrap_or(2));
    let now_ms = pii_core::SystemClock.now_ms();
    match add(&engine.memory, text, layer, now_ms) {
        Ok(added) => value_response(req, serde_json::to_value(added).ok(), err),
        Err(AddError::NoMarker) => err(
            ErrorCode::InvalidParams,
            "skill.put: the text has no PII-SKILL marker line",
        ),
        Err(AddError::Taken { slug, id }) => err(
            ErrorCode::InvalidParams,
            &format!("skill {slug} is already live as {id}; change it with pii skill edit"),
        ),
        Err(AddError::Store(e)) => err_code(err, &e),
    }
}

/// `skill.list {all?}`: the skill documents in the store, archived ones only when `all` is true. Answers a list of
/// `Listed` documents.
pub(crate) fn skill_list(engine: &Arc<Engine>, req: &Request) -> Response {
    let all = req
        .params
        .get("all")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let err = |c: ErrorCode, m: &str| Response::err(req.id.clone(), RpcError::new(c, m));
    match listing(&engine.memory, all) {
        Ok(skills) => value_response(req, serde_json::to_value(skills).ok(), err),
        Err(e) => err_code(err, &e),
    }
}
