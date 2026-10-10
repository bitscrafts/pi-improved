//! The pure part of a write: validate, decide created/merged/unchanged, enforce the caps.
//! It mutates the loaded [`State`] only when every rule has passed.

use super::store::{GraphConfig, Put, State, Written};
use super::types::{
    Entity, EntityInput, GraphError, Reason, Relationship, RelationshipInput, check_attributes,
    check_kind, check_name, check_provenance, entity_id, merge_attrs, merge_sources, refuse,
    relationship_id,
};

fn written(changed: bool, existed: bool) -> Written {
    match (existed, changed) {
        (false, _) => Written::Created,
        (true, true) => Written::Merged,
        (true, false) => Written::Unchanged,
    }
}

pub(super) fn upsert_entity(
    state: &mut State,
    cfg: &GraphConfig,
    input: EntityInput,
) -> Result<Put<Entity>, GraphError> {
    check_kind(&input.kind)?;
    check_name(&input.name)?;
    check_attributes(&input.attributes, cfg.max_attributes)?;
    let provenance = check_provenance(&input.provenance)?;
    let id = entity_id(&input.kind, &input.name);
    let existing = state.entities.get(&id);
    let record = if let Some(old) = existing {
        Entity {
            attributes: merge_attrs(&old.attributes, &input.attributes),
            provenance: merge_sources(&old.provenance, &provenance),
            ..old.clone()
        }
    } else {
        if state.entities.len() >= cfg.max_entities {
            return refuse(
                Reason::EntityCap,
                format!("the store holds {} entities", cfg.max_entities),
            );
        }
        Entity {
            id: id.clone(),
            kind: input.kind,
            name: input.name,
            attributes: input.attributes,
            provenance,
        }
    };
    check_attributes(&record.attributes, cfg.max_attributes)?;
    if record.provenance.len() > cfg.max_provenance {
        return refuse(
            Reason::ProvenanceCap,
            format!("entity {id} would exceed {} sources", cfg.max_provenance),
        );
    }
    let written = written(existing != Some(&record), existing.is_some());
    if written != Written::Unchanged {
        state.entities.insert(id, record.clone());
    }
    Ok(Put { record, written })
}

pub(super) fn upsert_relationship(
    state: &mut State,
    cfg: &GraphConfig,
    input: RelationshipInput,
) -> Result<Put<Relationship>, GraphError> {
    check_kind(&input.kind)?;
    check_attributes(&input.attributes, cfg.max_attributes)?;
    let provenance = check_provenance(&input.provenance)?;
    for (end, id) in [("from", &input.from), ("to", &input.to)] {
        if !state.entities.contains_key(id) {
            return refuse(
                Reason::UnknownEntity,
                format!("relationship '{end}' names no entity: {id}"),
            );
        }
    }
    let id = relationship_id(&input.from, &input.to, &input.kind);
    let existing = state.relationships.get(&id);
    let record = if let Some(old) = existing {
        Relationship {
            attributes: merge_attrs(&old.attributes, &input.attributes),
            provenance: merge_sources(&old.provenance, &provenance),
            ..old.clone()
        }
    } else {
        if state.relationships.len() >= cfg.max_relationships {
            return refuse(
                Reason::RelationshipCap,
                format!("the store holds {} relationships", cfg.max_relationships),
            );
        }
        Relationship {
            id: id.clone(),
            from: input.from,
            to: input.to,
            kind: input.kind,
            attributes: input.attributes,
            provenance,
        }
    };
    check_attributes(&record.attributes, cfg.max_attributes)?;
    if record.provenance.len() > cfg.max_provenance {
        return refuse(
            Reason::ProvenanceCap,
            format!(
                "relationship {id} would exceed {} sources",
                cfg.max_provenance
            ),
        );
    }
    let written = written(existing != Some(&record), existing.is_some());
    if written != Written::Unchanged {
        state.relationships.insert(id, record.clone());
    }
    Ok(Put { record, written })
}
