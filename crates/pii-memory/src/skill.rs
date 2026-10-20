//! The one naming rule for skill documents (spec 227 R1). `pii skill show <name>` and the `coder` node's skill store
//! both resolve a name here, so a skill is named the same way wherever it is read.
//!
//! The add path, the listing that `pii skill` and the daemon's `skill.put` / `skill.list` share, and the pin hash
//! live here too (spec 233).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use pii_abi::PiiError;
use pii_store::MemStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::doc::{Doc, DocKind, Layer, created_of, decode_kind, decode_text, rows_for};

/// The slug is the first non-empty line after the `PII-SKILL` marker.
#[must_use]
pub fn slug_of(text: &str) -> &str {
    let mut lines = text.lines();
    lines.by_ref().find(|l| l.trim() == "PII-SKILL");
    lines.map(str::trim).find(|l| !l.is_empty()).unwrap_or("-")
}

/// The lowercase sha256 of `bytes` as hex.
///
/// A graph's `skills = [{ name, sha256 }]` pin carries this value, and the coder node checks the stored text against
/// it, so there is one hash for both (spec 233 R1).
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Whether `text` has the `PII-SKILL` marker line that makes it a skill document.
#[must_use]
pub fn is_skill(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "PII-SKILL")
}

/// The live skill documents in `store`, as `(id, text)` in id order. Every other kind is skipped.
///
/// # Errors
/// [`PiiError::Internal`] on a redb failure.
pub fn live(store: &MemStore) -> Result<Vec<(String, String)>, PiiError> {
    let mut out = Vec::new();
    for id in store.doc_ids()? {
        let raw = store.doc(&id)?;
        if decode_kind(raw.clone()) == Ok(DocKind::Skill) {
            out.push((id, decode_text(raw)));
        }
    }
    Ok(out)
}

/// The items of `found` that `key` names: an exact id or slug wins, else a unique id prefix.
///
/// `of` gives an item's `(id, text)`. Several matches come back together, so the caller can refuse an ambiguous
/// name.
pub fn pick<'a, T>(found: &'a [T], key: &str, of: impl Fn(&T) -> (&str, &str)) -> Vec<&'a T> {
    let named: Vec<&T> = found
        .iter()
        .filter(|s| {
            let (id, text) = of(s);
            id == key || slug_of(text) == key
        })
        .collect();
    if !named.is_empty() {
        return named;
    }
    found
        .iter()
        .filter(|s| {
            let (id, _) = of(s);
            !key.is_empty() && id.starts_with(key)
        })
        .collect()
}

/// One skill document as the store holds it: its payload, layer, concept tags, and whether it is archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    /// The document id.
    pub id: String,
    /// The stored text, exactly as the store holds it.
    pub text: String,
    /// The layer tag (see `layer_byte`).
    pub layer: u8,
    /// Creation time in clock ms.
    pub created_ms: u64,
    /// The concept tags the document is filed under.
    pub concepts: Vec<String>,
    /// Whether the document is in the archive table.
    pub archived: bool,
}

/// Every skill document in `store`, archived ones only when `with_archived` is set (spec 233 R2). `pii skill` and the
/// daemon's `skill.list` both read through here, so they list the same skills.
///
/// # Errors
/// [`PiiError::Internal`] on a redb failure.
pub fn listing(store: &MemStore, with_archived: bool) -> Result<Vec<Listed>, PiiError> {
    let concepts = concepts_by_doc(store)?;
    let mut out = Vec::new();
    for id in store.doc_ids()? {
        out.extend(listed_of(store, &concepts, &id, false));
    }
    if with_archived {
        for id in store.archived_ids()? {
            out.extend(listed_of(store, &concepts, &id, true));
        }
    }
    Ok(out)
}

/// `concept -> doc ids` read back as `doc id -> concepts`. The store keeps edges per concept, so one pass over them is
/// the only way to ask a doc its concepts.
fn concepts_by_doc(store: &MemStore) -> Result<BTreeMap<String, Vec<String>>, PiiError> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for concept in store.concepts()? {
        for (doc, _) in store.edges(&concept)? {
            out.entry(doc).or_default().push(concept.clone());
        }
    }
    Ok(out)
}

/// Builds a [`Listed`] from a stored payload; `None` when the payload is not a skill document.
fn listed_of(
    store: &MemStore,
    concepts: &BTreeMap<String, Vec<String>>,
    id: &str,
    archived: bool,
) -> Option<Listed> {
    let raw = if archived {
        store.archived(id)
    } else {
        store.doc(id)
    }
    .ok()
    .flatten();
    if decode_kind(raw.clone()) != Ok(DocKind::Skill) {
        return None;
    }
    Some(Listed {
        id: id.to_owned(),
        text: decode_text(raw.clone()),
        layer: store.layer(id).ok().flatten().unwrap_or(0),
        created_ms: created_of(raw),
        concepts: concepts.get(id).cloned().unwrap_or_default(),
        archived,
    })
}

/// The document [`add`] stored: its id, its slug, and the sha256 of its text (the pin a graph carries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Added {
    /// The new document id.
    pub id: String,
    /// The slug the text names.
    pub slug: String,
    /// The sha256 of the stored text.
    pub sha256: String,
}

/// Why [`add`] stored nothing (spec 233 R1).
#[derive(Debug)]
pub enum AddError {
    /// The text has no `PII-SKILL` marker line.
    NoMarker,
    /// A live skill already holds the slug. Changing a skill is `pii skill edit`, not a second document.
    Taken {
        /// The slug the text names.
        slug: String,
        /// The id of the live skill that holds it.
        id: String,
    },
    /// The store failed.
    Store(PiiError),
}

/// Stores `text` as one skill document at `layer`. Refuses a text with no marker, and a slug a live skill already
/// holds, and writes nothing in either case (spec 233 R1).
///
/// # Errors
/// See [`AddError`].
pub fn add(store: &MemStore, text: &str, layer: Layer, now_ms: u64) -> Result<Added, AddError> {
    if !is_skill(text) {
        return Err(AddError::NoMarker);
    }
    let slug = slug_of(text).to_owned();
    let held = live(store).map_err(AddError::Store)?;
    if let Some((id, _)) = held.iter().find(|(_, t)| slug_of(t) == slug) {
        return Err(AddError::Taken {
            slug,
            id: id.clone(),
        });
    }
    let doc = Doc::new(layer, DocKind::Skill, text, Vec::new(), 1.0, now_ms);
    store.put(&rows_for(&doc)).map_err(AddError::Store)?;
    Ok(Added {
        id: doc.id.clone(),
        slug,
        sha256: sha256_hex(doc.text.as_bytes()),
    })
}
