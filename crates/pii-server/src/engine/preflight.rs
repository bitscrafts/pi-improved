//! Spec 288: a call's declared `needs` that the run input cannot supply refuses the run at start, before any run exists.
//!
//! The run-time `require` node (spec 267) still refuses a key that resolves only while the run executes. This check
//! decides a needs key the call maps from its `in` table only when that mapping is a plain run-input read
//! (`$.input.<name>`); any other mapped key is left to that node, as is every key of a nested call, whose `in` table
//! lives in a sub-graph's text.

use pii_abi::PiiError;
use pii_graph::fragment::CallRecord;
use toml::{Table, Value};

/// Refuses a run whose input lacks a non-null key that a call declares in `needs`: an unmapped key the input does not
/// supply, or a key mapped as `$.input.<name>` whose `<name>` the input does not supply.
///
/// The first missing key in `needs` order is named, with the run-time node's text. `host_src` is the host graph's
/// own text, where a top-level call's `in` table is read.
///
/// # Errors
/// [`PiiError::Invalid`] `graph.call.need` when such a key is absent or `null`.
pub fn check_needs(
    calls: &[CallRecord],
    host_src: &str,
    input: &serde_json::Value,
) -> Result<(), PiiError> {
    let host: Table = toml::from_str(host_src).unwrap_or_default();
    for call in calls {
        let Some(needs) = &call.needs else {
            continue;
        };
        for key in needs {
            let missing = if call.input_keys.contains(key) {
                // A mapped key is decided only by a plain run-input read; any other expression is for the run to decide.
                mapped_input_name(&host, call, key).is_some_and(|name| !supplied(input, name))
            } else {
                !supplied(input, key)
            };
            if missing {
                return Err(PiiError::Invalid(format!(
                    "graph.call.need: call '{}' graph '{}' needs '{key}'",
                    call.call, call.graph
                )));
            }
        }
    }
    Ok(())
}

/// True when the run input holds `name` with a non-null value (an empty string is a value).
fn supplied(input: &serde_json::Value, name: &str) -> bool {
    input.get(name).is_some_and(|v| !v.is_null())
}

/// The run-input name a top-level call maps `key` from, when its `in` expression is exactly `$.input.<name>`.
fn mapped_input_name<'a>(host: &'a Table, call: &CallRecord, key: &str) -> Option<&'a str> {
    if call.parent.is_some() {
        return None;
    }
    let expr = host
        .get("node")?
        .as_array()?
        .iter()
        .filter_map(Value::as_table)
        .find(|n| n.get("id").and_then(Value::as_str) == Some(call.call.as_str()))?
        .get("in")?
        .as_table()?
        .get(key)?
        .as_str()?;
    let name = expr.strip_prefix("$.input.")?;
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    plain.then_some(name)
}

/// Spec 397 R1: a `gate` that declares no `deadline_ms` refuses the start, naming the node.
///
/// # Errors
/// [`PiiError::Invalid`] naming the first gate without a positive `deadline_ms`.
pub fn check_gate_deadlines(spec: &pii_graph::GraphSpec) -> Result<(), PiiError> {
    let missing = spec.nodes.iter().find(|n| {
        n.kind == "gate"
            && n.cfg
                .get("deadline_ms")
                .and_then(serde_json::Value::as_u64)
                .is_none_or(|d| d == 0)
    });
    missing.map_or(Ok(()), |n| {
        Err(PiiError::Invalid(format!(
            "gate '{}' requires 'deadline_ms' (positive milliseconds it may wait for a decision)",
            n.id
        )))
    })
}
