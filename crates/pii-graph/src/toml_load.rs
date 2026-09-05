//! TOML graph loader (spec F4.2).

use pii_abi::{NodeId, PiiError, PortName};
use pii_core::CapKind;
use toml::Table;

use crate::spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};

/// Loads a graph spec from a TOML document (spec F4.2), defaulting each
/// node's caps from the **built-in** registry only.
///
/// # Errors
/// Returns [`PiiError::Invalid`] on a structural or type error in the TOML.
pub fn load_toml(src: &str) -> Result<GraphSpec, PiiError> {
    load_toml_with_registry(src, &crate::node::NodeRegistry::builtin())
}

/// Loads a graph spec from a TOML document (spec F4.2), defaulting each
/// node's caps from `registry` (B2, opus review).
///
/// **Why this exists**: [`load_toml`] alone defaults every node's caps from
/// [`crate::node::NodeRegistry::builtin`] — 4-5 kinds (`transform`, `route`,
/// `emit`, `wait`, `timer`). A caller that will actually *run* the graph
/// typically registers more kinds on top of that (e.g. `pii-server`
/// registers `agent`/`tool`/`gate`/`memory`/`crash`). If that caller loads
/// through plain [`load_toml`], every node of a non-builtin kind gets
/// [`CapKind::empty`] as its default caps — silently, because
/// [`crate::validate::validate`] checks caps against the *factory's*
/// declared `required_caps()` via its own registry, so lint stays green
/// while the runtime capset used by [`crate::dispatch`] is actually empty.
/// The fix is not a validator change: it is making sure the loader that
/// feeds a real execution passes the registry that will actually execute
/// it. Callers that only lint (no execution) may keep using [`load_toml`].
///
/// # Errors
/// Returns [`PiiError::Invalid`] on a structural or type error in the TOML.
#[allow(clippy::too_many_lines)]
pub fn load_toml_with_registry(
    src: &str,
    registry: &crate::node::NodeRegistry,
) -> Result<GraphSpec, PiiError> {
    let doc: Table =
        toml::from_str(src).map_err(|e| PiiError::Invalid(format!("graph toml parse: {e}")))?;

    let name = get_str(&doc, "name", "missing 'name'")?;

    // caps: optional array of capability names, default [].
    let caps: u32 = {
        let mut kinds = CapKind::empty();
        if let Some(toml::Value::Array(items)) = doc.get("caps") {
            for item in items {
                let s = item
                    .as_str()
                    .ok_or_else(|| PiiError::Invalid("caps entry must be a string".into()))?;
                kinds |= CapKind::parse_kind(s)?;
            }
        } else if let Some(other) = doc.get("caps") {
            return Err(PiiError::Invalid(format!(
                "'caps' must be an array, got {}",
                type_name(other)
            )));
        }
        kinds.bits()
    };

    let entry = match doc.get("entry") {
        Some(v) => NodeId::from(
            v.as_str()
                .ok_or_else(|| PiiError::Invalid("'entry' must be a string".into()))?,
        ),
        None => NodeId::from("unset"),
    };

    // limits
    let mut limits = GraphLimits::default();
    if let Some(toml::Value::Table(t)) = doc.get("limits") {
        if let Some(v) = t.get("max_steps") {
            let m: i64 = v
                .as_integer()
                .ok_or_else(|| PiiError::Invalid("max_steps must be an integer".into()))?;
            limits.max_steps =
                u32::try_from(m).map_err(|_| PiiError::Invalid("max_steps out of range".into()))?;
        }
        if let Some(v) = t.get("max_wall_ms") {
            let w: i64 = v
                .as_integer()
                .ok_or_else(|| PiiError::Invalid("max_wall_ms must be an integer".into()))?;
            limits.max_wall_ms = u64::try_from(w)
                .map_err(|_| PiiError::Invalid("max_wall_ms out of range".into()))?;
        }
        if let Some(v) = t.get("max_tokens") {
            let n: i64 = v
                .as_integer()
                .ok_or_else(|| PiiError::Invalid("max_tokens must be an integer".into()))?;
            limits.max_tokens = u64::try_from(n)
                .map_err(|_| PiiError::Invalid("max_tokens out of range".into()))?;
        }
        if let Some(v) = t.get("max_micro_usd") {
            let n: i64 = v
                .as_integer()
                .ok_or_else(|| PiiError::Invalid("max_micro_usd must be an integer".into()))?;
            limits.max_micro_usd = u64::try_from(n)
                .map_err(|_| PiiError::Invalid("max_micro_usd out of range".into()))?;
        }
        if let Some(v) = t.get("max_parallel") {
            let n: i64 = v
                .as_integer()
                .ok_or_else(|| PiiError::Invalid("max_parallel must be an integer".into()))?;
            limits.max_parallel = u32::try_from(n)
                .map_err(|_| PiiError::Invalid("max_parallel out of range".into()))?;
        }
    }

    // nodes
    let mut nodes = Vec::new();
    let mut ids_seen = std::collections::HashSet::new();
    if let Some(toml::Value::Array(arr)) = doc.get("node") {
        for item in arr {
            let t = item
                .as_table()
                .ok_or_else(|| PiiError::Invalid("each [[node]] must be a table".into()))?;
            let id = NodeId::from(get_str(t, "id", "node missing 'id'")?);
            if !ids_seen.insert(id.clone()) {
                return Err(PiiError::Invalid(format!("duplicate node id '{id}'")));
            }
            let kind = get_str(t, "kind", "node missing 'kind'")?;
            let kind = kind.to_string();
            let join = match t.get("join") {
                None => JoinPolicy::All,
                Some(v) => JoinPolicy::parse(
                    v.as_str()
                        .ok_or_else(|| PiiError::Invalid("'join' must be a string".into()))?,
                )?,
            };
            let loop_budget = match t.get("loop_budget") {
                None => None,
                Some(v) => Some(
                    u32::try_from(v.as_integer().ok_or_else(|| {
                        PiiError::Invalid("'loop_budget' must be an integer".into())
                    })?)
                    .map_err(|_| PiiError::Invalid("loop_budget out of range".into()))?,
                ),
            };
            let retry = match t.get("retry") {
                None => RetryPolicy::default(),
                Some(toml::Value::Table(rt)) => RetryPolicy {
                    max: u32::try_from(
                        rt.get("max")
                            .and_then(toml::Value::as_integer)
                            .ok_or_else(|| {
                                PiiError::Invalid("retry.max must be an integer".into())
                            })?,
                    )
                    .map_err(|_| PiiError::Invalid("retry.max out of range".into()))?,
                },
                Some(_) => return Err(PiiError::Invalid("'retry' must be a table".into())),
            };
            // caps: optional array; defaults to the kind's required_caps()
            // (spec part3: "Node caps default to the kind's required_caps()").
            // Unknown kinds keep empty — validate() rejects them separately.
            let caps = match t.get("caps") {
                None => registry.required_caps_of(&kind),
                Some(toml::Value::Array(items)) => {
                    let mut k = CapKind::empty();
                    for item in items {
                        let s = item.as_str().ok_or_else(|| {
                            PiiError::Invalid("node caps entry must be a string".into())
                        })?;
                        k |= CapKind::parse_kind(s)?;
                    }
                    k
                }
                Some(_) => return Err(PiiError::Invalid("node 'caps' must be an array".into())),
            };
            // cfg: every remaining key in the node table.
            let cfg = remaining_cfg(t);
            nodes.push(NodeSpec {
                id,
                kind,
                join,
                loop_budget,
                retry,
                caps: caps.bits(),
                cfg,
            });
        }
    }
    if nodes.is_empty() {
        return Err(PiiError::Invalid("graph has no [[node]] entries".into()));
    }

    // edges
    let mut edges = Vec::new();
    if let Some(toml::Value::Array(arr)) = doc.get("edge") {
        for item in arr {
            let t = item
                .as_table()
                .ok_or_else(|| PiiError::Invalid("each [[edge]] must be a table".into()))?;
            let from = NodeId::from(get_str(t, "from", "edge missing 'from'")?);
            let to = NodeId::from(get_str(t, "to", "edge missing 'to'")?);
            let port = PortName::from(get_str(t, "port", "edge missing 'port'")?);
            let kind = match t.get("kind") {
                Some(toml::Value::String(s)) if s == "order" => EdgeKind::Order,
                Some(toml::Value::String(s)) if s == "data" => EdgeKind::Data,
                None => EdgeKind::Data,
                Some(other) => {
                    return Err(PiiError::Invalid(format!(
                        "edge 'kind' must be data|order, got {}",
                        type_name(other)
                    )));
                }
            };
            edges.push(Edge {
                from,
                port,
                to,
                kind,
            });
        }
    }

    // Resolve default entry to first node.
    let entry = if entry.as_str() == "unset" {
        nodes
            .first()
            .ok_or_else(|| PiiError::Invalid("graph has no [[node]] entries".into()))?
            .id
            .clone()
    } else {
        entry
    };

    Ok(GraphSpec {
        name: name.to_string(),
        caps,
        nodes,
        edges,
        entry,
        limits,
    })
}

/// Remaining (non-reserved) keys in a node table become its `cfg` JSON.
fn remaining_cfg(t: &Table) -> serde_json::Value {
    use serde_json::{Map, Value};
    const RESERVED: &[&str] = &["id", "kind", "join", "loop_budget", "retry", "caps"];
    let mut map = Map::new();
    for (k, v) in t {
        let key = k.as_str();
        if RESERVED.contains(&key) {
            continue;
        }
        map.insert(k.clone(), toml_to_json(v));
    }
    Value::Object(map)
}

fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(_) => serde_json::Value::Null,
        toml::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(toml_to_json).collect::<Vec<_>>())
        }
        toml::Value::Table(t) => {
            let mut m = serde_json::Map::new();
            for (k, val) in t {
                m.insert(k.clone(), toml_to_json(val));
            }
            serde_json::Value::Object(m)
        }
    }
}

fn get_str<'a>(t: &'a Table, key: &str, err: &str) -> Result<&'a str, PiiError> {
    t.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| PiiError::Invalid(err.to_string()))
}

fn type_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::load_toml;
    use crate::spec::JoinPolicy;
    use pii_abi::NodeId;

    #[test]
    fn loads_diamond_graph() {
        let src = r#"
name = "diamond"
entry = "entry"
[limits]
max_steps = 1000
max_wall_ms = 600000

[[node]]
id = "entry"
kind = "transform"
expr = "$.input"

[[node]]
id = "a"
kind = "wait"
timer_ms = 150

[[node]]
id = "join"
kind = "transform"
join = "all"
expr = "$.payload"

[[edge]]
from = "entry"
port = "ok"
to = "a"
kind = "data"

[[edge]]
from = "a"
port = "fired"
to = "join"
kind = "data"
"#;
        let g = load_toml(src).unwrap();
        assert_eq!(g.name, "diamond");
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.entry, NodeId::from("entry"));
        assert_eq!(g.edges.len(), 2);
        let join = g.node(&NodeId::from("join")).unwrap();
        assert_eq!(join.join, JoinPolicy::All);
        // cfg picks up expr and timer_ms (non-reserved keys)
        assert!(join.cfg.get("expr").is_some());
    }

    #[test]
    fn rejects_duplicate_node() {
        let src = r#"
name = "d"
[[node]]
id = "a"
kind = "transform"
[[node]]
id = "a"
kind = "transform"
"#;
        assert!(load_toml(src).is_err());
    }

    #[test]
    fn default_entry_is_first_node() {
        let src = r#"
name = "d"
[[node]]
id = "z"
kind = "transform"
"#;
        let g = load_toml(src).unwrap();
        assert_eq!(g.entry, NodeId::from("z"));
    }
}
