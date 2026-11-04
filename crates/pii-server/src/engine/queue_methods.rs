//! `queue.add`, `queue.list`, `queue.claim` (spec 352 R1-R4): the daemon's queue, fed from outside.
//!
//! The daemon owns the queue store (single writer), so every write is a request here. `queue.claim` is the same
//! `claim_next` the graph's `select` node calls, so a dispatcher can drive the lane without a graph.

use std::sync::Arc;

use pii_observe::{Event, Level, names};
use pii_proto::{ErrorCode, QueueAddParams, QueueRow, Request, Response, RpcError};
use pii_store::queue::{Claim, WorkItem};

use crate::Engine;
use crate::engine::dispatch::{err_code, value_response};

/// The checks `queue.add` makes before anything is written (spec 352 R2): the params parse, the id is new in every
/// state, the spec path exists, and at least one criterion is given. A refusal is the whole response, and nothing
/// is written; an admitted row is returned for the dispatcher to store.
pub(crate) fn admit(engine: &Arc<Engine>, req: &Request) -> Result<WorkItem, (ErrorCode, String)> {
    let refuse = |c: ErrorCode, m: &str| Err((c, m.to_owned()));
    let Ok(p) = serde_json::from_value::<QueueAddParams>(req.params.clone()) else {
        return refuse(
            ErrorCode::InvalidParams,
            "queue.add needs id, spec, title, criteria[], touches[]",
        );
    };
    if p.id.is_empty() {
        return refuse(ErrorCode::InvalidParams, "queue.add needs an 'id'");
    }
    if p.criteria.is_empty() {
        let m = format!(
            "queue.add refused {}: no criteria, so the judge would have nothing to run",
            p.id
        );
        return refuse(ErrorCode::InvalidParams, &m);
    }
    match engine.queue.get_item(&p.id) {
        Ok(Some(prev)) => {
            let m = format!(
                "queue.add refused {}: already in the queue ({})",
                p.id,
                prev.status.name()
            );
            return refuse(ErrorCode::InvalidParams, &m);
        }
        Ok(None) => {}
        Err(e) => return refuse(ErrorCode::Internal, &e.to_string()),
    }
    if p.spec.is_empty() || !engine.cfg.workspace.join(&p.spec).exists() {
        let m = format!("queue.add refused {}: spec {} does not exist", p.id, p.spec);
        return refuse(ErrorCode::InvalidParams, &m);
    }
    let mut item = WorkItem::new(&p.id, &[]);
    item.spec = p.spec;
    item.title = p.title;
    item.criteria = p.criteria;
    item.touches = p.touches;
    Ok(item)
}

/// The answer to an admitted `queue.add`, once the dispatcher has stored the row: `{id}`, and the observe event.
pub(crate) fn added(engine: &Arc<Engine>, req: &Request, item: &WorkItem) -> Response {
    engine.log().event(
        &Event::new("engine", Level::Info, names::QUEUE_ITEM_ADDED).field("id", item.id.as_str()),
    );
    Response::ok(req.id.clone(), serde_json::json!({ "id": item.id }))
}

/// `queue.list`: every row, oldest first, as `{id, state, title, spec}`.
pub(crate) fn queue_list(engine: &Arc<Engine>, req: &Request) -> Response {
    let err = |c: ErrorCode, m: &str| Response::err(req.id.clone(), RpcError::new(c, m));
    match engine.queue.list_items() {
        Ok(items) => {
            let rows: Vec<QueueRow> = items
                .into_iter()
                .map(|i| QueueRow {
                    id: i.id,
                    state: i.status.name().to_owned(),
                    title: i.title,
                    spec: i.spec,
                })
                .collect();
            value_response(req, serde_json::to_value(rows).ok(), err)
        }
        Err(e) => err_code(err, &e),
    }
}

/// `queue.claim {by}`: claims the next pending item, answering it, or `null` when none is claimable.
pub(crate) fn queue_claim(engine: &Arc<Engine>, req: &Request) -> Response {
    let err = |c: ErrorCode, m: &str| Response::err(req.id.clone(), RpcError::new(c, m));
    let Some(by) = req.params.get("by").and_then(serde_json::Value::as_str) else {
        return err(ErrorCode::InvalidParams, "queue.claim needs 'by'");
    };
    match engine.queue.claim_next(by) {
        Ok(Claim::Claimed(item)) => {
            // Spec 356: a claim is a state change somebody may need to trace; the id and the holder, no payload.
            engine.log().event(
                &Event::new("queue", Level::Info, names::QUEUE_ITEM_CLAIMED)
                    .field("item", item.id.as_str())
                    .field("by", by),
            );
            value_response(req, serde_json::to_value(item).ok(), err)
        }
        Ok(_) => Response::ok(req.id.clone(), serde_json::Value::Null),
        Err(e) => err_code(err, &e),
    }
}
