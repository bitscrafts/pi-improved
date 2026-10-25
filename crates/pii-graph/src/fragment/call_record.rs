//! Spec 269: the record of each `call` node a load expanded. It is taken from what the loader verified (the pin it
//! checked, the caps it granted), not from the host's text, so a pin that does not hold leaves no record.

use pii_core::CapKind;
use toml::Table;

use super::call::Sub;

/// One expanded `call` node: the graph it inlined and the digest it verified, with its grant, its `needs`, its exit,
/// its mapped input keys, and the call that holds it when it is nested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    /// The call node's id in the graph it sits in.
    pub call: String,
    /// The graph the call inlined.
    pub graph: String,
    /// The sha256 the loader verified against the graph's file bytes.
    pub sha256: String,
    /// The capability names the call was granted.
    pub caps: Vec<String>,
    /// The sub-graph's declared `needs`; `None` when it declares none.
    pub needs: Option<Vec<String>>,
    /// The sub-graph's exit node.
    pub exit: String,
    /// The input keys the call maps from its `in` table.
    pub input_keys: Vec<String>,
    /// The id of the call that holds this one; `None` for a top-level call.
    pub parent: Option<String>,
}

/// The record of one call and the records of the calls its sub-graph made. A nested call that has no parent yet is a
/// direct child of this call, so it is named after it.
pub(super) fn of(
    call: &str,
    sha: &str,
    caps: CapKind,
    input: &Table,
    sub: &mut Sub,
) -> (CallRecord, Vec<CallRecord>) {
    let own = CallRecord {
        call: call.to_string(),
        graph: sub.graph.clone(),
        sha256: sha.to_string(),
        caps: caps.names().into_iter().map(str::to_string).collect(),
        needs: sub.needs.clone(),
        exit: sub.exit.clone(),
        input_keys: input.keys().cloned().collect(),
        parent: None,
    };
    let nested = std::mem::take(&mut sub.calls)
        .into_iter()
        .map(|mut r| {
            r.parent.get_or_insert_with(|| call.to_string());
            r
        })
        .collect();
    (own, nested)
}

/// The `call-from` mark a call's head node carries (spec 270 R1): the record's fields. `needs` and `parent` are
/// present only when they apply, never as null.
pub(super) fn call_from_fields(r: &CallRecord) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert("graph".into(), r.graph.clone().into());
    m.insert("sha256".into(), r.sha256.clone().into());
    m.insert("caps".into(), r.caps.clone().into());
    if let Some(needs) = &r.needs {
        m.insert("needs".into(), needs.clone().into());
    }
    m.insert("exit".into(), r.exit.clone().into());
    m.insert("input_keys".into(), r.input_keys.clone().into());
    if let Some(parent) = &r.parent {
        m.insert("parent".into(), parent.clone().into());
    }
    serde_json::Value::Object(m)
}
