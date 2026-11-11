//! Spec 255 and 261: the reference rewrite of an inlined sub-graph. Its `$.input` reads go to the call's own input
//! nodes, except a key the sub-graph declares in `needs` and the call does not map: that one reads the host run's
//! input (R2). Its `$.nodes.<m>` reads go to the namespaced copies, and a join's `$.payload.<arm>` to the namespaced
//! arm (R6). The rewrite governs `$.` expressions only. Rust-side reads of `NodeInput.input` (the coder's
//! `scope_root`, `heal_record`'s `signature` and `gate`) see the host's run input regardless. Split out of `call.rs`
//! to keep both under the 450-line gate.

use std::ops::Range;

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
            let next = rewrite_refs(s, refs);
            *s = next;
        }
        Value::Array(items) => items.iter_mut().for_each(|x| rewrite(x, refs)),
        Value::Table(t) => t.iter_mut().for_each(|(_, x)| rewrite(x, refs)),
        _ => {}
    }
}

/// Rewrites one field of an inlined node. Only a `prompt` or a refusal's `reason` string takes the placeholder rule
/// (spec 410) and an `expr` takes the reference scan (its `$.` may stand anywhere in the text); every other field keeps
/// `rewrite`, so a `system` or a `value` holding braces stays data (R2).
pub(super) fn rewrite_field(key: &str, v: &mut Value, refs: &Refs<'_>) {
    match v {
        Value::String(s) if key == "prompt" || key == "reason" => *s = rewrite_prompt(s, refs),
        Value::String(s) if key == "expr" => *s = rewrite_refs(s, refs),
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

/// Renames every `$.path` reference in an expression, wherever it stands, by the single-reference rule. The text between
/// references is copied byte for byte (R2): operators, spacing, and string literals, whose `$.` is data.
fn rewrite_refs(text: &str, refs: &Refs<'_>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for span in reference_spans(text) {
        out.push_str(&text[last..span.start]);
        out.push_str(&rewrite_expr(&text[span.clone()], refs));
        last = span.end;
    }
    out.push_str(&text[last..]);
    out
}

/// The byte ranges of the `$.path` references in an expression, in order. A `$.` inside a string literal is data and
/// is not a reference. The path runs over the parser's character set, so the renamer and the validator (`call_needs`)
/// read the same spans.
pub(super) fn reference_spans(text: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(off) = text[at..].find(['"', '$']) {
        let start = at + off;
        let rest = &text[start..];
        if rest.starts_with('"') {
            at = start + literal_end(rest);
        } else if let Some(body) = rest.strip_prefix("$.") {
            let name = body.find(|c: char| !is_path_char(c)).unwrap_or(body.len());
            at = start + 2 + name;
            spans.push(start..at);
        } else {
            at = start + 1;
        }
    }
    spans
}

/// The byte length of the string literal at the start of `text`, quotes included. A backslash escapes the next
/// character, so `\"` does not end it. An unterminated literal runs to the end of `text`.
fn literal_end(text: &str) -> usize {
    let mut escaped = false;
    for (i, c) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return i + 1;
        }
    }
    text.len()
}

/// The characters a reference's path may hold: the parser's set (`expr.rs`, `fn path`), so both agree on where it ends.
fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
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
