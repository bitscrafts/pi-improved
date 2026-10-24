//! Spec 257: the coder's config keys that the run input supplies. A designated key holding a `$.` expression resolves against
//! the run input; every other key stays literal, so the input never picks the env names, the image or the time bound.

use pii_abi::NodeInput;
use pii_graph::expr::eval_or_literal;

/// The keys the run input must supply a usable value for: a refusal names the first that does not.
const REQUIRED: [&str; 5] = ["agent", "model", "spec", "workdir", "brief"];

/// The keys the run input may supply; one that resolves to nothing is absent, as if the graph had not set it.
const OPTIONAL: [&str; 4] = ["provider", "session", "sid", "max_budget_usd"];

/// The `$.` expression a config value holds, if it holds one.
fn expr_of(value: &serde_json::Value) -> Option<&str> {
    value.as_str().filter(|s| s.starts_with("$."))
}

/// The effective config: `cfg` with each designated `$.` key resolved against `input`. The second value is the refusal
/// reason when a required key resolves to nothing usable; the caller refuses before anything spawns.
pub(super) fn resolve(
    cfg: &serde_json::Value,
    input: &NodeInput,
) -> (serde_json::Value, Option<String>) {
    let ctx = serde_json::json!({
        "input": input.input,
        "payload": input.payload,
        "nodes": input.nodes,
    });
    let mut out = cfg.clone();
    for &key in REQUIRED.iter().chain(OPTIONAL.iter()) {
        let Some(expr) = cfg.get(key).and_then(expr_of) else {
            continue;
        };
        let value = match eval_or_literal(&ctx, &serde_json::Value::String(expr.to_owned())) {
            Ok(value) => value,
            Err(e) => return (out, Some(format!("input key '{key}' ({expr}): {e}"))),
        };
        if REQUIRED.contains(&key) && value.as_str().is_none_or(str::is_empty) {
            return (
                out,
                Some(format!("input key '{key}' ({expr}) resolved to nothing")),
            );
        }
        if let Some(map) = out.as_object_mut() {
            if value.is_null() {
                map.remove(key);
            } else {
                map.insert(key.to_owned(), value);
            }
        }
    }
    (out, None)
}
