//! `remember` and `recall` node kinds (spec 126): a run writes a fact, a later run reads it back.
//!
//! Both go through the daemon's [`pii_memory::entity::EntityHub`], and both are attenuated by
//! the capability model: `remember` needs `memory_write`, `recall` needs `memory_read`, in the
//! graph's `caps` and on the node, or the run fails with `capability denied: memory: ...`.
//!
//! | node       | `op`            | arguments                                              | `ok` value                    |
//! |------------|-----------------|--------------------------------------------------------|-------------------------------|
//! | `remember` | `entity`        | `entity_kind`, `name`, `attributes`?, `provenance`?    | `{op, id, kind, outcome}`     |
//! | `remember` | `relationship`  | `from`, `to`, `rel_kind`, `attributes`?, `provenance`? | `{op, id, kind, outcome}`     |
//! | `recall`   | `get`           | `entity_id`                                                 | `{op, count, entities}`       |
//! | `recall`   | `by_kind`       | `entity_kind`, `limit`?                                | `{op, count, entities}`       |
//! | `recall`   | `by_name`       | `name`, `limit`?                                       | `{op, count, entities}`       |
//! | `recall`   | `neighbours`    | `entity_id`, `depth`?, `limit`?                        | `{op, count, entities, relationships}` |
//!
//! (`id` is taken: it names the node in the graph file.) `from`/`to` are an entity id or `{kind, name}`. Every argument may be a `$.` expression.
//! The writing run's id is always added to the provenance (`run:<id>`). A refused write or
//! query leaves on the `err` port as `{error, reason}`; it is already counted and reported.

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_abi::{Effects, NodeInput, Outcome, PiiError, PortName};
use pii_core::{CapKind, Ctx};
use pii_graph::expr::eval_or_literal;
use pii_graph::node::{GraphNode, NodeFactory};
use pii_memory::entity::{
    EntityHub, EntityInput, EntityStore, GraphError, RelationshipInput, entity_id,
};
use serde_json::{Value, json};

const REMEMBER_OPS: &[&str] = &["entity", "relationship"];
const RECALL_OPS: &[&str] = &["get", "by_kind", "by_name", "neighbours"];

/// Registered into [`crate::engine::registry`]; every `remember` node shares one hub.
pub struct RememberFactory {
    /// The shared entity-store slot.
    pub hub: Arc<EntityHub>,
}

/// Registered into [`crate::engine::registry`]; every `recall` node shares one hub.
pub struct RecallFactory {
    /// The shared entity-store slot.
    pub hub: Arc<EntityHub>,
}

fn op_of(kind: &str, cfg: &Value, known: &[&str]) -> Result<String, PiiError> {
    let op = cfg
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| PiiError::Invalid(format!("{kind} requires 'op' (string)")))?;
    if !known.contains(&op) {
        return Err(PiiError::Invalid(format!(
            "{kind}: unknown op '{op}' (known: {})",
            known.join(", ")
        )));
    }
    Ok(op.to_owned())
}

impl NodeFactory for RememberFactory {
    fn kind(&self) -> &'static str {
        "remember"
    }

    fn effects(&self) -> Effects {
        Effects::MUTATES_EXTERNAL
    }

    fn required_caps(&self) -> CapKind {
        CapKind::MEMORY_WRITE
    }

    fn ports(&self, _cfg: &Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let op = op_of("remember", cfg, REMEMBER_OPS)?;
        Ok(Box::new(Node {
            hub: Arc::clone(&self.hub),
            op,
            cfg: cfg.clone(),
            write: true,
        }))
    }
}

impl NodeFactory for RecallFactory {
    fn kind(&self) -> &'static str {
        "recall"
    }

    fn effects(&self) -> Effects {
        Effects::READS_FS
    }

    fn required_caps(&self) -> CapKind {
        CapKind::MEMORY_READ
    }

    fn ports(&self, _cfg: &Value) -> Result<Vec<PortName>, PiiError> {
        Ok(vec![PortName::from("ok"), PortName::from("err")])
    }

    fn build(&self, cfg: &Value) -> Result<Box<dyn GraphNode>, PiiError> {
        let op = op_of("recall", cfg, RECALL_OPS)?;
        Ok(Box::new(Node {
            hub: Arc::clone(&self.hub),
            op,
            cfg: cfg.clone(),
            write: false,
        }))
    }
}

struct Node {
    hub: Arc<EntityHub>,
    op: String,
    cfg: Value,
    write: bool,
}

/// One argument of the node config, with `$.` expressions resolved.
struct Args<'a> {
    cfg: &'a Value,
    ec: Value,
}

impl Args<'_> {
    fn get(&self, key: &str) -> Result<Option<Value>, PiiError> {
        self.cfg
            .get(key)
            .map(|v| eval_or_literal(&self.ec, v))
            .transpose()
    }

    fn text(&self, key: &str) -> Result<String, PiiError> {
        self.get(key)?
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| PiiError::Invalid(format!("this op requires '{key}' (string)")))
    }

    fn number<T: TryFrom<u64>>(&self, key: &str) -> Result<Option<T>, PiiError> {
        self.get(key)?
            .map(|v| {
                v.as_u64().and_then(|n| T::try_from(n).ok()).ok_or_else(|| {
                    PiiError::Invalid(format!("'{key}' must be a small unsigned integer"))
                })
            })
            .transpose()
    }

    fn attributes(&self) -> Result<BTreeMap<String, String>, PiiError> {
        let Some(v) = self.get("attributes")? else {
            return Ok(BTreeMap::new());
        };
        let obj = v
            .as_object()
            .ok_or_else(|| PiiError::Invalid(String::from("'attributes' must be an object")))?;
        obj.iter()
            .map(|(k, v)| {
                let v = eval_or_literal(&self.ec, v)?;
                Ok((
                    k.clone(),
                    v.as_str().map_or_else(|| v.to_string(), str::to_owned),
                ))
            })
            .collect()
    }

    fn sources(&self, run: &str) -> Result<Vec<String>, PiiError> {
        let mut out = vec![format!("run:{run}")];
        if let Some(v) = self.get("provenance")? {
            let list = v.as_array().ok_or_else(|| {
                PiiError::Invalid(String::from("'provenance' must be an array of strings"))
            })?;
            out.extend(list.iter().filter_map(|s| s.as_str().map(str::to_owned)));
        }
        Ok(out)
    }

    /// An entity reference: an id, or `{kind, name}`.
    fn entity_ref(&self, key: &str) -> Result<String, PiiError> {
        let v = self
            .get(key)?
            .ok_or_else(|| PiiError::Invalid(format!("this op requires '{key}'")))?;
        match &v {
            Value::String(s) => Ok(s.clone()),
            Value::Object(o) => {
                let f = |k: &str| {
                    o.get(k)
                        .map(|x| eval_or_literal(&self.ec, x))
                        .transpose()?
                        .and_then(|x| x.as_str().map(str::to_owned))
                        .ok_or_else(|| PiiError::Invalid(format!("'{key}.{k}' must be a string")))
                };
                Ok(entity_id(&f("kind")?, &f("name")?))
            }
            _ => Err(PiiError::Invalid(format!(
                "'{key}' must be an id or {{kind, name}}"
            ))),
        }
    }
}

fn refused(e: &GraphError) -> Outcome {
    Outcome::Next {
        port: PortName::from("err"),
        value: json!({"error": e.to_string(), "reason": e.reason()}),
    }
}

fn ok(value: Value) -> Outcome {
    Outcome::Next {
        port: PortName::from("ok"),
        value,
    }
}

fn failed(e: GraphError) -> Result<Outcome, PiiError> {
    match e {
        GraphError::Refused(..) => Ok(refused(&e)),
        GraphError::Store(s) => Err(PiiError::Internal(s)),
    }
}

impl Node {
    fn remember(&self, store: &EntityStore, a: &Args<'_>, run: &str) -> Result<Outcome, PiiError> {
        let provenance = a.sources(run)?;
        let attributes = a.attributes()?;
        if self.op == "entity" {
            let input = EntityInput {
                kind: a.text("entity_kind")?,
                name: a.text("name")?,
                attributes,
                provenance,
            };
            return match store.put_entity(input) {
                Ok(p) => Ok(ok(
                    json!({"op": "entity", "id": p.record.id, "kind": p.record.kind, "outcome": p.written.as_str()}),
                )),
                Err(e) => failed(e),
            };
        }
        let input = RelationshipInput {
            from: a.entity_ref("from")?,
            to: a.entity_ref("to")?,
            kind: a.text("rel_kind")?,
            attributes,
            provenance,
        };
        match store.put_relationship(input) {
            Ok(p) => Ok(ok(
                json!({"op": "relationship", "id": p.record.id, "kind": p.record.kind, "outcome": p.written.as_str()}),
            )),
            Err(e) => failed(e),
        }
    }

    fn recall(&self, store: &EntityStore, a: &Args<'_>) -> Result<Outcome, PiiError> {
        let limit: Option<usize> = a.number("limit")?;
        let found = match self.op.as_str() {
            "get" => store
                .get(&a.text("entity_id")?)
                .map(|e| e.into_iter().collect::<Vec<_>>()),
            "by_kind" => store.by_kind(&a.text("entity_kind")?, limit),
            "by_name" => store.by_name(&a.text("name")?, limit),
            _ => {
                return match store.neighbours(&a.text("entity_id")?, a.number("depth")?, limit) {
                    Ok(n) => Ok(ok(json!({
                        "op": "neighbours",
                        "count": n.entities.len(),
                        "entities": n.entities.iter().map(|(d, e)| json!({"depth": d, "entity": e})).collect::<Vec<_>>(),
                        "relationships": n.relationships,
                    }))),
                    Err(e) => failed(e),
                };
            }
        };
        match found {
            Ok(list) => Ok(ok(
                json!({"op": self.op, "count": list.len(), "entities": list}),
            )),
            Err(e) => failed(e),
        }
    }
}

impl GraphNode for Node {
    fn run(&self, ctx: &Ctx, input: &NodeInput) -> Result<Outcome, PiiError> {
        let (need, verb) = if self.write {
            (CapKind::MEMORY_WRITE, "write")
        } else {
            (CapKind::MEMORY_READ, "read")
        };
        if !ctx.caps().kinds.contains(need) {
            return Err(PiiError::Denied(format!(
                "memory: {verb} not granted (the graph and the node need the memory_{verb} capability)"
            )));
        }
        let store = self.hub.require()?;
        let a = Args {
            cfg: &self.cfg,
            ec: json!({"input": input.input, "payload": input.payload, "nodes": input.nodes}),
        };
        if self.write {
            self.remember(&store, &a, &ctx.trace().run.to_string())
        } else {
            self.recall(&store, &a)
        }
    }
}
