//! `graph.index` and `graph.find` (spec 388).
//!
//! The daemon holds the memory store, so it is the one to write and search the catalogue while it runs. The entries are
//! derived by the caller, from files the caller names.

use std::sync::Arc;

use pii_abi::{PiiError, RunEvent};
use pii_core::Clock as _;
use pii_memory::catalogue::{Entry, find, sync};
use pii_memory::embed::TrigramEmbedder;
use pii_memory::search::MemorySearcher;
use pii_proto::{ErrorCode, Request, Response};
use pii_store::Store;

use crate::Engine;
use crate::engine::dispatch::{err_code, value_response};
use crate::engine::methods::err;

/// The journal kind of an agent's `graph_find` call, with its question and what it answered.
pub const GRAPH_ASKED: &str = "graph.asked";
/// The journal kind of an agent's write to a graph file.
pub const GRAPH_WRITTEN: &str = "graph.written";

/// `graph.find { question }`. Holds nothing a file read would not, so it asks for no capability.
pub(crate) fn graph_find(engine: &Arc<Engine>, req: &Request) -> Response {
    let Some(question) = req.params.get("question").and_then(|v| v.as_str()) else {
        return err(req, ErrorCode::InvalidParams, "graph.find needs 'question'");
    };
    let searcher = MemorySearcher::new(Arc::clone(&engine.memory), Some(Arc::new(TrigramEmbedder)))
        .with_observer(engine.log());
    let ctx = pii_core::Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited());
    match find(&searcher, &engine.memory, &ctx, question) {
        Ok(answer) => value_response(req, serde_json::to_value(answer).ok(), |c, m| {
            err(req, c, m)
        }),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// `graph.index { entries }`.
pub(crate) fn graph_index(engine: &Arc<Engine>, req: &Request) -> Response {
    let entries: Vec<Entry> = match req
        .params
        .get("entries")
        .cloned()
        .map(serde_json::from_value)
    {
        Some(Ok(entries)) => entries,
        _ => return err(req, ErrorCode::InvalidParams, "graph.index needs 'entries'"),
    };
    let now_ms = pii_core::SystemClock.now_ms();
    let done = sync(&engine.memory, &entries, now_ms).and_then(|synced| {
        Ok(serde_json::json!({
            "synced": synced,
            "graphs_written_without_a_lookup": graphs_written_without_a_lookup(&engine.store)?,
        }))
    });
    match done {
        Ok(reply) => value_response(req, Some(reply), |c, m| err(req, c, m)),
        Err(e) => err_code(|c, m| err(req, c, m), &e),
    }
}

/// R5: the runs that wrote a graph with no `graph_find` before the write. The journal is the only witness, so an index
/// run, which journals nothing, can never count as a lookup.
///
/// # Errors
/// [`PiiError::Internal`] on a store failure.
pub fn graphs_written_without_a_lookup(store: &Store) -> Result<u64, PiiError> {
    let mut runs = Vec::new();
    store.for_each_run_header(|header| runs.push(header.run))?;
    let mut count = 0;
    for run in &runs {
        let mut asked = false;
        for (_, event) in store.read_journal(run)? {
            let RunEvent::NodeObserved { kind, .. } = event else {
                continue;
            };
            if kind == GRAPH_ASKED {
                asked = true;
            } else if kind == GRAPH_WRITTEN && !asked {
                count += 1;
                break;
            }
        }
    }
    Ok(count)
}
