//! Spec 255 and 261: the reference rewrite of an inlined sub-graph. Its `$.input` reads go to the call's own input
//! nodes, except a key the sub-graph declares in `needs` and the call does not map: that one reads the host run's
//! input (R2). Its `$.nodes.<m>` reads go to the namespaced copies, and a join's `$.payload.<arm>` to the namespaced
//! arm (R6). The rewrite governs `$.` expressions only. Rust-side reads of `NodeInput.input` (the coder's
//! `scope_root`, `heal_record`'s `signature` and `gate`) see the host's run input regardless. Split out of `call.rs`
//! to keep both under the 450-line gate.

use toml::Value;

/// The sub-graph ids, input keys, declaration and join arms a call's references are rewritten against.
pub(super) struct Refs<'a> {
    pub(super) call: &'a str,
    pub(super) keys: &'a [String],
    pub(super) nodes: &'a [String],
    pub(super) needs: Option<&'a [String]>,
    pub(super) arms: &'a [String],
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
        ["input", key, ..]
            if refs
                .needs
                .is_some_and(|n| n.iter().any(|d| d.as_str() == *key)) =>
        {
            expr.to_string()
        }
        ["input", rest @ ..] => nodes_path(&format!("{}__in", refs.call), rest),
        ["nodes", m, rest @ ..] if refs.nodes.iter().any(|n| n.as_str() == *m) => {
            nodes_path(&format!("{}__{m}", refs.call), rest)
        }
        ["payload", m, rest @ ..] if refs.arms.iter().any(|a| a.as_str() == *m) => {
            path("payload", &format!("{}__{m}", refs.call), rest)
        }
        _ => expr.to_string(),
    }
}

fn nodes_path(first: &str, rest: &[&str]) -> String {
    path("nodes", first, rest)
}

fn path(root: &str, first: &str, rest: &[&str]) -> String {
    let mut out = format!("$.{root}.{first}");
    for seg in rest {
        out.push('.');
        out.push_str(seg);
    }
    out
}
