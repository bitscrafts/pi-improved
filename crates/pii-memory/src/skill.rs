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

use crate::doc::{
    Doc, DocKind, Layer, created_of, decode_kind, decode_text, layer_of_byte, rows_for,
};

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

/// One `script: <path> sha256=<hex>` line from a skill body. An empty `sha256` never matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// The script path, relative to the directory the pin is read from.
    pub path: String,
    /// The pinned sha256 as written in the body.
    pub sha256: String,
}

/// Every `script: <path> sha256=<hex>` line of a body, in order.
#[must_use]
pub fn pins_of(body: &str) -> Vec<Pin> {
    body.lines()
        .filter_map(|line| {
            let mut parts = line.trim().strip_prefix("script:")?.split_whitespace();
            let path = parts.next()?.to_owned();
            let sha256 = parts
                .find_map(|t| t.strip_prefix("sha256="))
                .unwrap_or("")
                .to_owned();
            Some(Pin { path, sha256 })
        })
        .collect()
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

/// The document [`edit`] stored: its id (unchanged), its new slug, the sha256 of its new text (the pin a graph must
/// move to), and the body's length before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edited {
    /// The document id, which the edit keeps.
    pub id: String,
    /// The slug the new text names.
    pub slug: String,
    /// The sha256 of the new text.
    pub sha256: String,
    /// The old text's length in bytes.
    pub bytes_before: u64,
    /// The new text's length in bytes.
    pub bytes_after: u64,
}

/// Why [`edit`] stored nothing (spec 234 R2).
#[derive(Debug)]
pub enum EditError {
    /// No live skill document holds the id.
    NoLive {
        /// The id that was asked for.
        id: String,
    },
    /// The new text has no `PII-SKILL` marker line.
    NoMarker,
    /// The new text drops a pin the old text carried. De-pinning needs a new skill id, which `add` makes.
    DropsPin {
        /// The script paths the old text pinned and the new text does not.
        paths: Vec<String>,
    },
    /// The store failed.
    Store(PiiError),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLive { id } => write!(f, "no live skill with id {id}"),
            Self::NoMarker => {
                f.write_str("the text has no PII-SKILL marker line, so it is not a skill")
            }
            Self::DropsPin { paths } => write!(
                f,
                "refused: the edit would drop the pin for {}; de-pinning needs a NEW skill id; body unchanged",
                paths.join(", ")
            ),
            Self::Store(e) => write!(f, "{e}"),
        }
    }
}

/// Replaces the text of a live skill, keeping its id.
///
/// The layer, concepts and creation time are kept. The concept edges are not re-put: they are already in the graph.
/// `source` is recorded as the doc's evidence (spec 210). Refuses, and writes nothing, when no live skill holds `id`,
/// when `new_text` has no marker, or when it drops a pin the old text carried (spec 234 R1, R2). `pii skill edit` and
/// the daemon's `skill.edit` both call this.
///
/// # Errors
/// See [`EditError`].
pub fn edit(
    store: &MemStore,
    id: &str,
    new_text: &str,
    now_ms: u64,
    source: &str,
) -> Result<Edited, EditError> {
    let old = listing(store, false)
        .map_err(EditError::Store)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| EditError::NoLive { id: id.to_owned() })?;
    if !is_skill(new_text) {
        return Err(EditError::NoMarker);
    }
    let new_pins = pins_of(new_text);
    let paths: Vec<String> = pins_of(&old.text)
        .into_iter()
        .filter(|p| !new_pins.iter().any(|n| n.path == p.path))
        .map(|p| p.path)
        .collect();
    if !paths.is_empty() {
        return Err(EditError::DropsPin { paths });
    }

    let bytes_before = u64::try_from(old.text.len()).unwrap_or(u64::MAX);
    let mut doc = Doc::new(
        layer_of_byte(old.layer),
        DocKind::Skill,
        new_text,
        old.concepts,
        1.0,
        old.created_ms,
    );
    doc.id.clone_from(&old.id);
    let mut rows = rows_for(&doc);
    rows.edges.clear();
    store.put(&rows).map_err(EditError::Store)?;
    store
        .set_evidence(id, source, now_ms)
        .map_err(EditError::Store)?;
    Ok(Edited {
        id: old.id,
        slug: slug_of(new_text).to_owned(),
        sha256: sha256_hex(new_text.as_bytes()),
        bytes_before,
        bytes_after: u64::try_from(new_text.len()).unwrap_or(u64::MAX),
    })
}
