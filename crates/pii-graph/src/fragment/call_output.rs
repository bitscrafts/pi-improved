//! Spec 343 R2: a host reads a called graph's terminal value **by the call's id**.
//!
//! Inclusion is inline at load (spec 255): the called graph's nodes become `<call>__<node>`, and its exit keeps its
//! own id as `<call>__<exit>`. A host that wants the called graph's result should not have to know which inner
//! node produced it, so every host expression that mentions the call id is rewritten here:
//!
//! ```text
//! $.nodes.sdd        ->  $.nodes.sdd__record
//! $.nodes.sdd.verdict ->  $.nodes.sdd__record.verdict
//! ```
//!
//! Only the HOST's own nodes are walked, and only their values — never `id`, `kind`, or the called graph's own
//! nodes (which are already namespaced and may legitimately mention an id that looks like a call).
//!
//! A prefix must end at a `.` or the end of the string: `$.nodes.sdd` rewrites, `$.nodes.sddx` does not, and a
//! host that never mentions the call id comes out byte-identical — which is the control this module is tested by.

use toml::Value;

/// Rewrites `$.nodes.<call>` and `$.nodes.<call>.<rest>` to `$.nodes.<call>__<exit>[.<rest>]` in every host node's
/// values. Returns how many expressions were rewritten.
pub(super) fn alias_outputs(doc: &mut toml::Table, call: &str, exit: &str) -> usize {
    let prefix = format!("$.nodes.{call}");
    let replacement = format!("$.nodes.{call}__{exit}");
    let mut rewritten = 0;
    if let Some(Value::Array(nodes)) = doc.get_mut("node") {
        for node in nodes.iter_mut() {
            let Value::Table(t) = node else { continue };
            for (key, value) in t.iter_mut() {
                if key == "id" || key == "kind" {
                    continue;
                }
                rewritten += rewrite_value(value, &prefix, &replacement);
            }
        }
    }
    rewritten
}

fn rewrite_value(value: &mut Value, prefix: &str, replacement: &str) -> usize {
    match value {
        Value::String(s) => rewrite_str(s, prefix, replacement).map_or(0, |new| {
            *s = new;
            1
        }),
        Value::Array(items) => items
            .iter_mut()
            .map(|v| rewrite_value(v, prefix, replacement))
            .sum(),
        Value::Table(t) => t
            .iter_mut()
            .map(|(_, v)| rewrite_value(v, prefix, replacement))
            .sum(),
        _ => 0,
    }
}

/// `Some(rewritten)` when `s` mentions the call id as a whole path segment, `None` otherwise.
fn rewrite_str(s: &str, prefix: &str, replacement: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut hit = false;
    while let Some(i) = rest.find(prefix) {
        let after = &rest[i + prefix.len()..];
        // a segment boundary: the end of the string, or a `.` — `$.nodes.sddx` is a different node
        if after.is_empty() || after.starts_with('.') {
            out.push_str(&rest[..i]);
            out.push_str(replacement);
            rest = after;
            hit = true;
        } else {
            out.push_str(&rest[..i + prefix.len()]);
            rest = after;
        }
    }
    out.push_str(rest);
    hit.then_some(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_call_id_is_rewritten_to_the_exit() {
        assert_eq!(
            rewrite_str("$.nodes.sdd", "$.nodes.sdd", "$.nodes.sdd__record").as_deref(),
            Some("$.nodes.sdd__record")
        );
    }

    #[test]
    fn a_field_of_the_call_value_keeps_its_field() {
        assert_eq!(
            rewrite_str("$.nodes.sdd.verdict", "$.nodes.sdd", "$.nodes.sdd__record").as_deref(),
            Some("$.nodes.sdd__record.verdict")
        );
    }

    #[test]
    fn a_longer_id_that_starts_with_the_call_id_is_not_rewritten() {
        // the near-miss: `sddx` is a different node, and silently rewriting it would read the wrong value
        assert_eq!(
            rewrite_str("$.nodes.sddx", "$.nodes.sdd", "$.nodes.sdd__record"),
            None
        );
        assert_eq!(
            rewrite_str("$.nodes.sddx.verdict", "$.nodes.sdd", "$.nodes.sdd__record"),
            None
        );
    }

    #[test]
    fn an_unrelated_expression_is_untouched() {
        assert_eq!(
            rewrite_str("$.input.row", "$.nodes.sdd", "$.nodes.sdd__record"),
            None
        );
    }

    #[test]
    fn a_node_id_and_kind_are_never_rewritten() {
        let mut doc: toml::Table = toml::from_str(
            r#"
            [[node]]
            id = "sdd"
            kind = "call"
            expr = "$.nodes.sdd"
            "#,
        )
        .expect("toml");
        let n = alias_outputs(&mut doc, "sdd", "record");
        assert_eq!(n, 1, "only the expression is rewritten");
        let node = &doc["node"].as_array().expect("nodes")[0];
        assert_eq!(node["id"].as_str(), Some("sdd"), "the id is untouched");
        assert_eq!(node["kind"].as_str(), Some("call"), "the kind is untouched");
        assert_eq!(node["expr"].as_str(), Some("$.nodes.sdd__record"));
    }
}
