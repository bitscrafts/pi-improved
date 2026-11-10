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

/// Rewrites one field of an inlined node. Only a `prompt` string takes the placeholder rule; every other field keeps
/// `rewrite`, so a `system` or a `value` holding braces stays data (R2).
pub(super) fn rewrite_field(key: &str, v: &mut Value, refs: &Refs<'_>) {
    match v {
        Value::String(s) if key == "prompt" => *s = rewrite_prompt(s, refs),
        _ => rewrite(v, refs),
    }
}

/// Renames each `{{path}}` placeholder by the rule its `$.path` expression follows. A placeholder that is not renamed
/// keeps its text byte for byte, spacing included (R3); one that names no sub-graph node stays as written (R4).
fn rewrite_prompt(text: &str, refs: &Refs<'_>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("{{") {
        out.push_str(before);
        out.push_str("{{");
        let Some((inner, tail)) = after.split_once("}}") else {
            out.push_str(after);
            return out;
        };
        let body = inner.trim();
        let start = inner.len() - inner.trim_start().len();
        let (lead, body_and_trail) = inner.split_at(start);
        let closing_ws = body_and_trail.strip_prefix(body).unwrap_or_default();
        let next = rewrite_expr(&format!("$.{body}"), refs);
        out.push_str(lead);
        out.push_str(next.strip_prefix("$.").unwrap_or(body));
        out.push_str(closing_ws);
        out.push_str("}}");
        rest = tail;
    }
    out.push_str(rest);
    out
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
