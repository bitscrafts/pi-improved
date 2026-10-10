//! Entities and relationships (spec 126): the memory pillar's structure.
//!
//! Typed entities and relationships with content-derived ids, mandatory provenance,
//! idempotent upsert, a stated referential-integrity rule (a relationship naming an unknown
//! entity is refused), bounded queries, and a hub that lets the engine's `remember` and
//! `recall` nodes share one store. No model is involved: records are written by code from
//! run data, and everything in a name or an attribute is data.
//!
//! See `_docs/02-guides/memory-entities.md`.

mod archive;
mod hub;
mod query;
mod store;
mod tier;
mod types;
mod upsert;

pub use archive::{ArchiveReport, RestoreReport};
pub use hub::{EntityHealth, EntityHub};
pub use query::Neighbourhood;
pub use store::{CompactReport, Counters, Counts, EntityStore, GraphConfig, Put, Written};
pub use tier::{Access, Ranked, Tier, TierInfo};
pub use types::{
    Entity, EntityInput, GraphError, MAX_ATTR_KEY_BYTES, MAX_ATTR_VALUE_BYTES, MAX_KIND_BYTES,
    MAX_NAME_BYTES, MAX_SOURCE_BYTES, Reason, Relationship, RelationshipInput, entity_id,
    is_entity_id, relationship_id,
};
