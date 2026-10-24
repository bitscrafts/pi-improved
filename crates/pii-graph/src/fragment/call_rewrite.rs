//! Spec 255: the reference rewrite of an inlined sub-graph. Its `$.input` reads go to the call's own input nodes
//! and its `$.nodes.<m>` reads go to the namespaced copies, so no inlined node can read the host's input or a host
//! node. Split out of `call.rs` to keep both under the 450-line gate.

use toml::Value;

/// The sub-graph ids and input keys a call's references are rewritten against.
pub(super) struct Refs<'a> {
    pub(super) call: &'a str,
    pub(super) keys: &'a [String],
    pub(super) nodes: &'a [String],
}

/// Rewrites one value's expressions: `$.input` to the call's inputs, `$.nodes.<m>` to the namespaced copy.
pub(super) fn rewrite(v: &mut Value, refs: &Refs<'_>) {
    match v {
        Value::String(s) if s.starts_with("$.") => {
            let next = rewrite_expr(s, refs);
            *s = next;
        }
        Value::Array(items) => items.iter_mut().for_each(|x| rewrite(x, refs)),
        Value::Table(t) => t.iter_mut().for_each(|(_, x)| rewrite(x, refs)),
        _ => {}
    }
}

fn rewrite_expr(expr: &str, refs: &Refs<'_>) -> String {
    let segs: Vec<&str> = expr[2..].split('.').collect();
    match segs.as_slice() {
        ["input", key, rest @ ..] if refs.keys.iter().any(|k| k.as_str() == *key) => {
            nodes_path(&format!("{}__in__{key}", refs.call), rest)
        }
        ["input", rest @ ..] => nodes_path(&format!("{}__in", refs.call), rest),
        ["nodes", m, rest @ ..] if refs.nodes.iter().any(|n| n.as_str() == *m) => {
            nodes_path(&format!("{}__{m}", refs.call), rest)
        }
        _ => expr.to_string(),
    }
}

fn nodes_path(first: &str, rest: &[&str]) -> String {
    let mut out = format!("$.nodes.{first}");
    for seg in rest {
        out.push('.');
        out.push_str(seg);
    }
    out
}
