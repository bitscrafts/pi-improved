//! Spec 261: a sub-graph declares the run-input keys it needs (`needs`), and the loader checks its `$.` reads against
//! that declaration and against the host's nodes before the rewrite. An undeclared input key, a bare `$.input` and a
//! host node read are refused; a nested call's read of its holder's input must be declared by the holder. A join's
//! arms are found here too. Split out of `call.rs` to keep both under the 450-line gate.

use pii_abi::PiiError;
use toml::{Table, Value};

use super::call::{id_of, tables};

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
            let segs: Vec<&str> = expr[2..].split('.').collect();
            match segs.as_slice() {
                ["nodes", m, ..]
                    if host_ids.iter().any(|h| h == *m) && !scope.ids.iter().any(|s| s == *m) =>
                {
                    return Err(PiiError::Invalid(format!(
                        "graph.call.leak: graph '{}' node '{nid}' reads host node '{m}' through call '{}'; map it with 'in'",
                        scope.graph, scope.call
                    )));
                }
                ["input"] if scope.needs.is_some() => {
                    return Err(refusal(scope, &nid, "the whole input", declared));
                }
                ["input", key, ..] if scope.needs.is_some() && !scope.input.contains_key(*key) => {
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

/// The `$.` strings of a node, in the places the rewrite visits them: every key but `id`, nested.
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
        if k != "id" {
            walk(v, &mut out);
        }
    }
    out
}
