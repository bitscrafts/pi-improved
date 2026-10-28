//! Spec 255: a `call` node inlines another graph at load time.
//!
//! [`expand_host`] runs on the host table before the weave. Each `kind = "call"` node is replaced by the named
//! graph's nodes, ids namespaced `<call>__<n>`, behind a `<call>__in` head. The sub-graph's `$.input` reads go to
//! the call's own input nodes and its `$.nodes.<m>` reads go to its own copies, so it never sees the host's input.
//! Each input key the call maps is its own transform, `<call>__in__<key>`, chained after the head: a transform
//! evaluates only a top-level `$.` string, so one node cannot hold a whole object of expressions.
//!
//! Spec 261: a sub-graph that declares `needs` reads its undeclared-but-unmapped keys from the host run's input, and
//! [`super::call_needs`] refuses every other input or host-node read before the rewrite. The rewrite governs `$.`
//! expressions only; Rust-side reads of `NodeInput.input` (the coder's `scope_root`, `heal_record`) see the host's
//! run input regardless, and a node with no inbound Data edge gets the run input as its payload.
//!
//! Spec 306: a call's head counts its entries, and the run fails once a call is entered more than `max_entries` times
//! (default 8). The count is cumulative for the run and never reset by an enclosing loop, so an inner call inside a
//! looped outer call reaches its own limit after that many entries across all the outer passes; the author raises the
//! inner `max_entries` explicitly when that is too few.

use std::path::PathBuf;

use pii_abi::PiiError;
use pii_core::CapKind;
use toml::{Table, Value};

use crate::node::NodeRegistry;

use super::call_needs::{self, Scope};
use super::call_record::{self, CallRecord};
use super::call_rewrite::{Refs, rewrite};
use super::check::sha256_hex;

/// The deepest chain of nested calls a load accepts (spec 255 R7).
const MAX_DEPTH: usize = 4;

/// The entries a call may make before the run fails, when the call gives no `max_entries` (spec 306 R1).
const DEFAULT_MAX_ENTRIES: i64 = 8;

/// The keys a `call` node may carry (spec 255, spec 261 R7, spec 306 R4).
const CALL_KEYS: [&str; 7] = ["id", "kind", "graph", "sha256", "caps", "in", "max_entries"];

/// A sub-graph read from its file, expanded, and checked against its call.
pub(super) struct Sub {
    pub(super) graph: String,
    pub(super) needs: Option<Vec<String>>,
    pub(super) exit: String,
    entry: String,
    ids: Vec<String>,
    nodes: Vec<Table>,
    edges: Vec<Table>,
    /// The records of the calls this sub-graph made (spec 269), in expansion order.
    pub(super) calls: Vec<CallRecord>,
}

/// True when the table holds a `call` node. A fragment may not hold one (spec 255 R9).
pub(super) fn has_call(doc: &Table) -> bool {
    doc.get("node")
        .and_then(Value::as_array)
        .is_some_and(|nodes| {
            nodes
                .iter()
                .filter_map(Value::as_table)
                .any(|n| kind_of(n) == Some("call"))
        })
}

/// Expands every `call` node of the host source. Returns `None` when the source has no call, so the caller loads
/// the original text unchanged; otherwise the expanded graph as TOML.
///
/// # Errors
/// [`PiiError::Invalid`] naming the `graph.call.*` rule that failed, or a `fragment.*` rule for a bad pin.
pub(super) fn expand_host(
    src: &str,
    graph_dirs: &[PathBuf],
    reg: &NodeRegistry,
) -> Result<(Option<String>, Vec<CallRecord>), PiiError> {
    // A source that does not parse is left for the loader, which names the parse error.
    let Ok(mut doc) = toml::from_str::<Table>(src) else {
        return Ok((None, Vec::new()));
    };
    if !has_call(&doc) {
        return Ok((None, Vec::new()));
    }
    let name = doc
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let grant = caps_of(doc.get("caps"))?;
    let records = expand(&mut doc, graph_dirs, reg, &mut vec![name], grant)?;
    let text = toml::to_string(&doc).map_err(|e| PiiError::Invalid(format!("graph toml: {e}")))?;
    Ok((Some(text), records))
}

/// Expands every call in `doc`. `chain` is the graph names from the host down to `doc`; `grant` is the most
/// capability `doc`'s calls may ask for (the host's caps, or the caps the enclosing call granted).
fn expand(
    doc: &mut Table,
    graph_dirs: &[PathBuf],
    reg: &NodeRegistry,
    chain: &mut Vec<String>,
    grant: CapKind,
) -> Result<Vec<CallRecord>, PiiError> {
    if graph_dirs.is_empty() {
        return Err(PiiError::Invalid(
            "graph.call.no_dir: a call node needs a graph directory to resolve its graph in; no graph directory was given"
                .into(),
        ));
    }
    // The first node may be a call, so the entry is named before any node is replaced.
    if let Some(first) = first_id(doc).filter(|_| !doc.contains_key("entry")) {
        doc.insert("entry".into(), Value::String(first));
    }
    let mut records = Vec::new();
    while let Some(call) = take_call(doc) {
        records.extend(expand_one(doc, &call, graph_dirs, reg, chain, grant)?);
    }
    Ok(records)
}

/// Replaces one call node: checks its parts, reads its graph, and splices the graph's nodes and edges in. Returns the
/// records of this call and of the calls its graph made.
fn expand_one(
    doc: &mut Table,
    call: &Table,
    graph_dirs: &[PathBuf],
    reg: &NodeRegistry,
    chain: &mut Vec<String>,
    grant: CapKind,
) -> Result<Vec<CallRecord>, PiiError> {
    let id = id_of(call)
        .ok_or_else(|| PiiError::Invalid("graph.call.missing: a call has no 'id'".into()))?;
    if let Some(k) = call.keys().find(|k| !CALL_KEYS.contains(&k.as_str())) {
        return Err(PiiError::Invalid(format!(
            "graph.call.key: call '{id}' has unknown key '{k}'"
        )));
    }
    let graph = required(call, &id, "graph")?;
    let sha = required(call, &id, "sha256")?;
    let call_caps = caps_of(Some(call.get("caps").ok_or_else(|| missing(&id, "caps"))?))?;
    let input = match call.get("in") {
        None => Table::new(),
        Some(v) => v.as_table().cloned().ok_or_else(|| {
            PiiError::Invalid(format!(
                "graph.call.in: call '{id}' must give 'in' as a table"
            ))
        })?,
    };

    let max_entries = max_entries_of(call, &id)?;
    let widen = call_caps & !grant;
    if !widen.is_empty() {
        return Err(PiiError::Invalid(format!(
            "graph.call.widen: call '{id}' grants {} the graph it sits in does not hold",
            names(widen)
        )));
    }
    if chain.contains(&graph) {
        return Err(PiiError::Invalid(format!(
            "graph.call.cycle: {} -> {graph}",
            chain.join(" -> ")
        )));
    }
    if chain.len() > MAX_DEPTH {
        return Err(PiiError::Invalid(format!(
            "graph.call.depth: call '{id}' nests past the depth cap of {MAX_DEPTH}, via {} -> {graph}",
            chain.join(" -> ")
        )));
    }

    let mut sub = load_sub(&id, &graph, &sha, call_caps, graph_dirs, reg, chain)?;
    let (own, nested) = call_record::of(&id, &sha, call_caps, &input, &mut sub);
    // The nested heads are marked now, with their parents, so the mark each carries names the call that holds it.
    call_needs::mark(&mut sub.nodes, &nested)?;
    let scope = Scope {
        call: &id,
        graph: &sub.graph,
        needs: sub.needs.as_deref(),
        input: &input,
        ids: &sub.ids,
        nodes: &sub.nodes,
    };
    call_needs::check(&scope, doc, chain)?;
    splice(doc, &id, &input, sub, &own, max_entries)?;
    Ok(std::iter::once(own).chain(nested).collect())
}

/// Reads the sub-graph file, checks its pin, expands its own calls, and checks its exit and its caps against the
/// call's grant.
fn load_sub(
    id: &str,
    graph: &str,
    sha: &str,
    call_caps: CapKind,
    graph_dirs: &[PathBuf],
    reg: &NodeRegistry,
    chain: &mut Vec<String>,
) -> Result<Sub, PiiError> {
    let path = find(graph_dirs, graph)?;
    let bytes = std::fs::read(&path)
        .map_err(|e| PiiError::Invalid(format!("graph.call.read: {}: {e}", path.display())))?;
    let actual = sha256_hex(&bytes);
    if actual != sha {
        return Err(PiiError::Invalid(format!(
            "fragment.pin.mismatch: call '{id}' pins graph '{graph}' to sha256 {sha}, but its file is {actual}"
        )));
    }
    let text = std::str::from_utf8(&bytes).map_err(|e| {
        PiiError::Invalid(format!(
            "graph.call.read: graph '{graph}' is not UTF-8: {e}"
        ))
    })?;
    let mut doc: Table = toml::from_str(text)
        .map_err(|e| PiiError::Invalid(format!("graph toml parse: graph '{graph}': {e}")))?;
    let needs = call_needs::parse(&doc, graph)?;

    chain.push(graph.to_string());
    let expanded = expand(&mut doc, graph_dirs, reg, chain, call_caps);
    chain.pop();
    let calls = expanded?;

    let exit = doc
        .get("exit")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            PiiError::Invalid(format!(
                "graph.call.exit: graph '{graph}' declares no 'exit' node"
            ))
        })?;
    let nodes = tables(&doc, "node");
    let ids: Vec<String> = nodes.iter().filter_map(id_of).collect();
    if !ids.contains(&exit) {
        return Err(PiiError::Invalid(format!(
            "graph.call.exit: graph '{graph}' exit '{exit}' names no node"
        )));
    }
    let entry = doc
        .get("entry")
        .and_then(Value::as_str)
        .map_or_else(|| ids.first().cloned().unwrap_or_default(), str::to_string);

    for node in &nodes {
        let nid = id_of(node).unwrap_or_default();
        let needs = match node.get("caps") {
            Some(v) => caps_of(Some(v))?,
            None => reg.required_caps_of(kind_of(node).unwrap_or("")),
        };
        let excess = needs & !call_caps;
        if !excess.is_empty() {
            return Err(PiiError::Invalid(format!(
                "graph.call.widen: node '{nid}' of graph '{graph}' needs {} beyond the {} call '{id}' grants",
                names(excess),
                names(call_caps)
            )));
        }
    }
    Ok(Sub {
        graph: graph.to_string(),
        needs,
        exit,
        entry,
        ids,
        nodes,
        edges: tables(&doc, "edge"),
        calls,
    })
}

/// The most entries a call may make (spec 306 R1, R4): an integer of at least 1, or the default when absent.
fn max_entries_of(call: &Table, id: &str) -> Result<i64, PiiError> {
    call.get("max_entries").map_or(Ok(DEFAULT_MAX_ENTRIES), |v| {
        v.as_integer().filter(|n| *n >= 1).ok_or_else(|| {
            PiiError::Invalid(format!(
                "graph.call.max_entries: call '{id}' must give 'max_entries' as an integer of at least 1"
            ))
        })
    })
}

/// Adds the call's head, input nodes and sub-graph to the host, rewires the host edges that touched the call, and
/// refuses an id the host already has. The head carries the call's `max_entries`, which the run counts (spec 306).
fn splice(
    doc: &mut Table,
    id: &str,
    input: &Table,
    sub: Sub,
    own: &CallRecord,
    max_entries: i64,
) -> Result<(), PiiError> {
    let keys: Vec<String> = input.keys().cloned().collect();
    let head = format!("{id}__in");
    let entry = format!("{id}__{}", sub.entry);
    let (mut added, mut new_edges) =
        call_needs::chain(id, &sub.graph, sub.needs.as_deref(), input, &entry);
    // `chain` puts the head first.
    if let Some(first) = added.first_mut() {
        first.insert("max_entries".into(), Value::Integer(max_entries));
    }
    call_needs::mark(&mut added, std::slice::from_ref(own))?;

    for mut node in sub.nodes {
        let nid = id_of(&node).unwrap_or_default();
        node.insert("id".into(), Value::String(format!("{id}__{nid}")));
        let arms = call_needs::arms(&sub.edges, &nid);
        let refs = Refs {
            call: id,
            keys: &keys,
            nodes: &sub.ids,
            needs: sub.needs.as_deref(),
            arms: &arms,
        };
        for (k, v) in &mut node {
            if k != "id" {
                rewrite(v, &refs);
            }
        }
        added.push(node);
    }
    for mut e in sub.edges {
        let from = e
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let to = e
            .get("to")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        e.insert("from".into(), Value::String(format!("{id}__{from}")));
        e.insert("to".into(), Value::String(format!("{id}__{to}")));
        new_edges.push(e);
    }

    let host_ids: Vec<String> = tables(doc, "node").iter().filter_map(id_of).collect();
    if let Some(clash) = added
        .iter()
        .filter_map(id_of)
        .find(|nid| host_ids.contains(nid))
    {
        return Err(PiiError::Invalid(format!(
            "graph.call.id: call '{id}' adds node '{clash}', which the host already has"
        )));
    }

    // Host edges that touched the call now touch its head (in) or its exit.
    if doc.get("entry").and_then(Value::as_str) == Some(id) {
        doc.insert("entry".into(), Value::String(head.clone()));
    }
    let mut host_edges = tables(doc, "edge");
    for e in &mut host_edges {
        if e.get("to").and_then(Value::as_str) == Some(id) {
            e.insert("to".into(), Value::String(head.clone()));
        }
        if e.get("from").and_then(Value::as_str) == Some(id) {
            e.insert("from".into(), Value::String(format!("{id}__{}", sub.exit)));
        }
    }
    host_edges.extend(new_edges);

    let mut nodes = take_array(doc, "node");
    nodes.extend(added.into_iter().map(Value::Table));
    doc.insert("node".into(), Value::Array(nodes));
    doc.insert(
        "edge".into(),
        Value::Array(host_edges.into_iter().map(Value::Table).collect()),
    );
    Ok(())
}

/// Removes and returns the first `call` node of `doc`.
fn take_call(doc: &mut Table) -> Option<Table> {
    let nodes = doc.get_mut("node")?.as_array_mut()?;
    let at = nodes
        .iter()
        .position(|n| n.as_table().and_then(kind_of) == Some("call"))?;
    nodes.remove(at).try_into().ok()
}

/// Resolves a graph name to its file. A name is a bare file stem: no separator, no leading dot.
fn find(graph_dirs: &[PathBuf], graph: &str) -> Result<PathBuf, PiiError> {
    if graph.is_empty() || graph.starts_with('.') || graph.contains(['/', '\\']) {
        return Err(PiiError::Invalid(format!(
            "graph.call.name: '{graph}' is not a graph name"
        )));
    }
    graph_dirs
        .iter()
        .map(|d| d.join(format!("{graph}.toml")))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            PiiError::Invalid(format!(
                "graph.call.not_found: graph '{graph}' is in none of the {} graph dirs",
                graph_dirs.len()
            ))
        })
}

/// The capabilities a `caps` value names. Absent means none; a non-array is refused.
fn caps_of(v: Option<&Value>) -> Result<CapKind, PiiError> {
    let Some(v) = v else {
        return Ok(CapKind::empty());
    };
    let items = v
        .as_array()
        .ok_or_else(|| PiiError::Invalid("'caps' must be an array".into()))?;
    let mut kinds = CapKind::empty();
    for item in items {
        let s = item
            .as_str()
            .ok_or_else(|| PiiError::Invalid("caps entry must be a string".into()))?;
        kinds |= CapKind::parse_kind(s)?;
    }
    Ok(kinds)
}

fn names(kinds: CapKind) -> String {
    kinds.names().join(",")
}

fn required(call: &Table, id: &str, key: &str) -> Result<String, PiiError> {
    call.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| missing(id, key))
}

fn missing(id: &str, key: &str) -> PiiError {
    PiiError::Invalid(format!("graph.call.missing: call '{id}' has no '{key}'"))
}

pub(super) fn id_of(node: &Table) -> Option<String> {
    node.get("id").and_then(Value::as_str).map(str::to_string)
}

fn first_id(doc: &Table) -> Option<String> {
    doc.get("node")
        .and_then(Value::as_array)
        .and_then(|n| n.first())
        .and_then(Value::as_table)
        .and_then(id_of)
}

fn kind_of(node: &Table) -> Option<&str> {
    node.get("kind").and_then(Value::as_str)
}

/// The tables under `key` in `doc`, cloned. Absent or non-table entries give none.
pub(super) fn tables(doc: &Table, key: &str) -> Vec<Table> {
    doc.get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|v| v.as_table().cloned()).collect())
        .unwrap_or_default()
}

/// Takes the array under `key` out of `doc` (empty when absent); the caller puts it back.
fn take_array(doc: &mut Table, key: &str) -> Vec<Value> {
    match doc.remove(key) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}
