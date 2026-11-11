//! Spec 261: a sub-graph declares the run-input keys it needs (`needs`), and the loader checks its `$.` reads against
//! that declaration and against the host's nodes before the rewrite. An undeclared input key, a bare `$.input` and a
//! host node read are refused; a nested call's read of its holder's input must be declared by the holder. A join's
//! arms are found here too. Split out of `call.rs` to keep both under the 450-line gate.

use pii_abi::PiiError;
use toml::{Table, Value};

use super::call::{id_of, tables};
use super::call_record::{CallRecord, call_from_fields};
use super::call_rewrite::reference_spans;

/// What one call checks in its sub-graph: the sub-graph's name and declaration, the call's `in` table, and the
/// sub-graph's node ids and nodes as they stand before the rewrite.
pub(super) struct Scope<'a> {
    pub(super) call: &'a str,
    pub(super) graph: &'a str,
    pub(super) needs: Option<&'a [String]>,
    pub(super) input: &'a Table,
    pub(super) ids: &'a [String],
    pub(super) nodes: &'a [Table],
}

/// The keys a graph declares in `needs`: `None` when it declares none (spec 255), otherwise an array of distinct
/// non-empty strings.
pub(super) fn parse(doc: &Table, graph: &str) -> Result<Option<Vec<String>>, PiiError> {
    let Some(v) = doc.get("needs") else {
        return Ok(None);
    };
    let malformed = || {
        PiiError::Invalid(format!(
            "graph.call.needs: graph '{graph}' has a malformed 'needs'"
        ))
    };
    let mut keys: Vec<String> = Vec::new();
    for item in v.as_array().ok_or_else(malformed)? {
        let key = item
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(malformed)?;
        if keys.iter().any(|k| k == key) {
            return Err(malformed());
        }
        keys.push(key.to_string());
    }
    Ok(Some(keys))
}

/// Checks one call's sub-graph. `host` is the graph the call sits in and `chain` the graph names from the root host
/// down to it: a holder below the root must declare each input key its nested call reads (spec 261 R8).
pub(super) fn check(scope: &Scope<'_>, host: &Table, chain: &[String]) -> Result<(), PiiError> {
    let host_ids: Vec<String> = tables(host, "node").iter().filter_map(id_of).collect();
    let declared = scope.needs.unwrap_or(&[]);
    // A key the sub-graph declares and the call does not map: a read of the host run's input, to be checked upward.
    let mut passed: Vec<(String, String)> = Vec::new();
    for node in scope.nodes {
        let nid = id_of(node).unwrap_or_default();
        for expr in node_exprs(node) {
            for span in reference_spans(expr) {
                let segs: Vec<&str> = expr[span][2..].split('.').collect();
                match segs.as_slice() {
                    ["nodes", m, ..]
                        if host_ids.iter().any(|h| h == *m)
                            && !scope.ids.iter().any(|s| s == *m) =>
                    {
                        return Err(PiiError::Invalid(format!(
                            "graph.call.leak: graph '{}' node '{nid}' reads host node '{m}' through call '{}'; map it with 'in'",
                            scope.graph, scope.call
                        )));
                    }
                    ["input"] if scope.needs.is_some() => {
                        return Err(refusal(scope, &nid, "the whole input", declared));
                    }
                    ["input", key, ..]
                        if scope.needs.is_some() && !scope.input.contains_key(*key) =>
                    {
                        if !declared.iter().any(|d| d == *key) {
                            let read = format!("input '{key}'");
                            return Err(refusal(scope, &nid, &read, declared));
                        }
                        passed.push((nid.clone(), key.to_string()));
                    }
                    _ => {}
                }
            }
        }
    }

    if chain.len() > 1 && !passed.is_empty() {
        let holder = chain.last().map_or("", String::as_str);
        let holder_needs = parse(host, holder)?.unwrap_or_default();
        for (node, key) in &passed {
            if !holder_needs.iter().any(|d| d == key) {
                let read = format!("input '{key}'");
                return Err(PiiError::Invalid(format!(
                    "graph.call.undeclared: graph '{holder}' node '{node}' reads {read} for call '{}', which the holder does not declare; holder declares only [{}]",
                    scope.call,
                    holder_needs.join(", ")
                )));
            }
        }
    }
    Ok(())
}

/// The inbound Data-edge sources of `node`, when it has more than one: a join, whose payload is keyed by source
/// (spec 261 R6). A node with one or no inbound Data edge has no arms; its payload is a direct value.
pub(super) fn arms(edges: &[Table], node: &str) -> Vec<String> {
    let srcs: Vec<String> = edges
        .iter()
        .filter(|e| e.get("kind").and_then(Value::as_str).unwrap_or("data") == "data")
        .filter(|e| e.get("to").and_then(Value::as_str) == Some(node))
        .filter_map(|e| e.get("from").and_then(Value::as_str).map(str::to_string))
        .collect();
    if srcs.len() > 1 { srcs } else { Vec::new() }
}

/// A data edge from `from` on its `ok` port to `to`.
pub(super) fn edge(from: &str, to: &str) -> Table {
    let mut t = Table::new();
    t.insert("from".into(), Value::String(from.to_string()));
    t.insert("port".into(), Value::String("ok".into()));
    t.insert("to".into(), Value::String(to.to_string()));
    t.insert("kind".into(), Value::String("data".into()));
    t
}

/// The `require` node `<id>__need` of a call whose sub-graph declares `needs` (spec 267 R2): one key per declared key.
/// A key the call's `in` maps (`mapped`) reads its `<id>__in__<key>` transform; any other key reads the host run's input.
pub(super) fn require_node(id: &str, graph: &str, needs: &[String], mapped: &[String]) -> Table {
    let keys: Table = needs
        .iter()
        .map(|key| {
            let expr = if mapped.contains(key) {
                format!("$.nodes.{id}__in__{key}")
            } else {
                format!("$.input.{key}")
            };
            (key.clone(), Value::String(expr))
        })
        .collect();
    let mut t = Table::new();
    t.insert("id".into(), Value::String(format!("{id}__need")));
    t.insert("kind".into(), Value::String("require".into()));
    t.insert("call".into(), Value::String(id.into()));
    t.insert("graph".into(), Value::String(graph.into()));
    t.insert("keys".into(), Value::Table(keys));
    t
}

/// The call's input chain (spec 255, spec 267 R2): the `<id>__in` head, one `<id>__in__<key>` transform per mapped
/// key, then, when the sub-graph declares `needs`, the `<id>__need` node, then the sub-graph `entry`. Returns the nodes
/// and the edges to add.
pub(super) fn chain(
    id: &str,
    graph: &str,
    needs: Option<&[String]>,
    input: &Table,
    entry: &str,
) -> (Vec<Table>, Vec<Table>) {
    let head = format!("{id}__in");
    let mut added = vec![transform(&head, Value::Table(Table::new()))];
    let mut edges = Vec::new();
    let mut prev = head;
    for (key, value) in input {
        let key_id = format!("{id}__in__{key}");
        added.push(transform(&key_id, value.clone()));
        edges.push(edge(&prev, &key_id));
        prev = key_id;
    }
    match needs {
        None => edges.push(edge(&prev, entry)),
        Some(needs) => {
            let mapped: Vec<String> = input.keys().cloned().collect();
            let need = format!("{id}__need");
            added.push(require_node(id, graph, needs, &mapped));
            edges.push(edge(&prev, &need));
            edges.push(edge(&need, entry));
        }
    }
    (added, edges)
}

/// A transform node `id` that holds `value`.
fn transform(id: &str, value: Value) -> Table {
    let mut t = Table::new();
    t.insert("id".into(), Value::String(id.to_string()));
    t.insert("kind".into(), Value::String("transform".into()));
    t.insert("value".into(), value);
    t
}

/// The `graph.call.undeclared` refusal for a read of `read` in node `nid` of the sub-graph of `scope`.
fn refusal(scope: &Scope<'_>, nid: &str, read: &str, declared: &[String]) -> PiiError {
    let maps: Vec<&str> = scope.input.keys().map(String::as_str).collect();
    PiiError::Invalid(format!(
        "graph.call.undeclared: graph '{}' node '{nid}' reads {read} but declares only [{}] and call '{}' maps [{}]",
        scope.graph,
        declared.join(", "),
        scope.call,
        maps.join(", ")
    ))
}

/// The strings of a node the rewrite reads references in, in the places it visits them (`call.rs`, `rewrite_field`): an
/// `expr` field whole, wherever its `$.` stands, and every other key but `id` by its nested `$.` strings.
fn node_exprs(node: &Table) -> Vec<&str> {
    fn walk<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
        match v {
            Value::String(s) if s.starts_with("$.") => out.push(s),
            Value::Array(items) => items.iter().for_each(|x| walk(x, out)),
            Value::Table(t) => t.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (k, v) in node {
        match v {
            Value::String(s) if k == "expr" => out.push(s.as_str()),
            _ if k != "id" => walk(v, &mut out),
            _ => {}
        }
    }
    out
}

/// Stamps the head node (`<call>__in`) of each record in `records` with its `call-from` mark (spec 270 R1), built from
/// the verified record. A head `nodes` does not hold is left alone.
pub(super) fn mark(nodes: &mut [Table], records: &[CallRecord]) -> Result<(), PiiError> {
    for r in records {
        let head = format!("{}__in", r.call);
        let Some(node) = nodes
            .iter_mut()
            .find(|n| n.get("id").and_then(Value::as_str) == Some(head.as_str()))
        else {
            continue;
        };
        let fields = Value::try_from(call_from_fields(r))
            .map_err(|e| PiiError::Invalid(format!("graph.call.mark: call '{}': {e}", r.call)))?;
        node.insert("call-from".into(), fields);
    }
    Ok(())
}
