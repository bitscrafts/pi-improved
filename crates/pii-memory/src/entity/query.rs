//! The bounded query surface. Every path is capped, every result is sorted, and a result
//! that does not fit the cap is a named refusal (`result_cap`), never a silent truncation.
//!
//! Cost: the graph is loaded whole (it is bounded by `max_entities`/`max_relationships`),
//! so `by_kind` and `by_name` are one pass over at most `max_entities` records, and
//! `neighbours` builds an adjacency over at most `max_relationships` links.

use std::collections::{BTreeMap, BTreeSet};

use super::store::EntityStore;
use super::types::{Entity, GraphError, Reason, Relationship, refuse};

/// The entities around one entity, nearest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbourhood {
    /// The entity the walk started from.
    pub root: String,
    /// `(depth, entity)` for every entity reached, sorted by depth then id; the root is not included.
    pub entities: Vec<(u32, Entity)>,
    /// The relationships the walk crossed, sorted by id.
    pub relationships: Vec<Relationship>,
}

impl EntityStore {
    fn limit(&self, limit: Option<usize>) -> Result<usize, GraphError> {
        let cap = self.config().max_results;
        match limit {
            None => Ok(cap),
            Some(0) => refuse(Reason::BadQuery, "limit must be at least 1"),
            Some(n) if n > cap => refuse(
                Reason::ResultCap,
                format!("limit {n} exceeds the result cap {cap}"),
            ),
            Some(n) => Ok(n),
        }
    }

    /// One entity by id; `None` when there is none.
    ///
    /// # Errors
    /// [`GraphError::Store`] when the file cannot be read.
    pub fn get(&self, id: &str) -> Result<Option<Entity>, GraphError> {
        let found = self.load()?.entities.get(id).cloned();
        if found.is_some() {
            self.record_access(&[id])?;
        }
        Ok(found)
    }

    /// Entities of `kind`, sorted by id.
    ///
    /// # Errors
    /// `bad_query` (empty kind), `result_cap` (more matches than the limit); or a store failure.
    pub fn by_kind(&self, kind: &str, limit: Option<usize>) -> Result<Vec<Entity>, GraphError> {
        self.select("by_kind", kind, limit, |e| e.kind == kind)
    }

    /// Entities named exactly `name` (any kind), sorted by id. Substring search is not offered.
    ///
    /// # Errors
    /// As [`EntityStore::by_kind`].
    pub fn by_name(&self, name: &str, limit: Option<usize>) -> Result<Vec<Entity>, GraphError> {
        self.select("by_name", name, limit, |e| e.name == name)
    }

    fn select(
        &self,
        op: &'static str,
        key: &str,
        limit: Option<usize>,
        keep: impl Fn(&Entity) -> bool,
    ) -> Result<Vec<Entity>, GraphError> {
        let found = self.select_untouched(op, key, limit, keep)?;
        let ids: Vec<&str> = found.iter().map(|e| e.id.as_str()).collect();
        self.record_access(&ids)?;
        Ok(found)
    }

    /// The matching entities, sorted by id, without recording a read.
    pub(super) fn select_untouched(
        &self,
        op: &'static str,
        key: &str,
        limit: Option<usize>,
        keep: impl Fn(&Entity) -> bool,
    ) -> Result<Vec<Entity>, GraphError> {
        let run = || {
            if key.trim().is_empty() {
                return refuse(Reason::BadQuery, "the query is empty");
            }
            let limit = self.limit(limit)?;
            let state = self.load()?;
            // The map is sorted by id, so the matches come out sorted.
            let mut out = Vec::new();
            for e in state.entities.values().filter(|e| keep(e)) {
                if out.len() == limit {
                    return refuse(
                        Reason::ResultCap,
                        format!("more than {limit} matches; narrow the query"),
                    );
                }
                out.push(e.clone());
            }
            Ok(out)
        };
        run().map_err(|e| self.refuse_query(op, e))
    }

    /// The entities within `depth` links of `id`, following links in either direction.
    /// `depth` defaults to the configured default and may not exceed `max_depth`.
    ///
    /// # Errors
    /// `unknown_entity`, `bad_depth` (zero), `depth_cap`, `result_cap`; or a store failure.
    pub fn neighbours(
        &self,
        id: &str,
        depth: Option<u32>,
        limit: Option<usize>,
    ) -> Result<Neighbourhood, GraphError> {
        let run = || {
            let cfg = self.config();
            let depth = depth.unwrap_or(cfg.default_depth);
            if depth == 0 {
                return refuse(Reason::BadDepth, "depth must be at least 1");
            }
            if depth > cfg.max_depth {
                return refuse(
                    Reason::DepthCap,
                    format!("depth {depth} exceeds the cap {}", cfg.max_depth),
                );
            }
            let limit = self.limit(limit)?;
            let state = self.load()?;
            if !state.entities.contains_key(id) {
                return refuse(Reason::UnknownEntity, format!("no entity {id}"));
            }
            let mut adj: BTreeMap<&str, Vec<&Relationship>> = BTreeMap::new();
            for r in state.relationships.values() {
                adj.entry(r.from.as_str()).or_default().push(r);
                adj.entry(r.to.as_str()).or_default().push(r);
            }
            let mut seen: BTreeSet<&str> = BTreeSet::from([id]);
            let mut crossed: BTreeMap<&str, &Relationship> = BTreeMap::new();
            let mut reached: Vec<(u32, &str)> = Vec::new();
            let mut frontier = vec![id];
            for d in 1..=depth {
                let mut next = Vec::new();
                for node in &frontier {
                    for r in adj.get(node).into_iter().flatten() {
                        let other = if r.from == *node {
                            r.to.as_str()
                        } else {
                            r.from.as_str()
                        };
                        crossed.insert(r.id.as_str(), r);
                        if seen.insert(other) {
                            if reached.len() == limit {
                                return refuse(
                                    Reason::ResultCap,
                                    format!(
                                        "more than {limit} neighbours; lower the depth or raise nothing"
                                    ),
                                );
                            }
                            reached.push((d, other));
                            next.push(other);
                        }
                    }
                }
                frontier = next;
            }
            reached.sort_unstable();
            let entities = reached
                .into_iter()
                .filter_map(|(d, i)| state.entities.get(i).map(|e| (d, e.clone())))
                .collect();
            Ok(Neighbourhood {
                root: id.to_owned(),
                entities,
                relationships: crossed.into_values().cloned().collect(),
            })
        };
        let found = run().map_err(|e| self.refuse_query("neighbours", e))?;
        let ids: Vec<&str> = std::iter::once(found.root.as_str())
            .chain(found.entities.iter().map(|(_, e)| e.id.as_str()))
            .chain(found.relationships.iter().map(|r| r.id.as_str()))
            .collect();
        self.record_access(&ids)?;
        Ok(found)
    }
}
