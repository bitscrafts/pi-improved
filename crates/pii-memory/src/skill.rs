//! The one naming rule for skill documents (spec 227 R1). `pii skill show <name>` and the `coder` node's skill store
//! both resolve a name here, so a skill is named the same way wherever it is read.

use pii_abi::PiiError;
use pii_store::MemStore;

use crate::doc::{DocKind, decode_kind, decode_text};

/// The slug is the first non-empty line after the `PII-SKILL` marker.
#[must_use]
pub fn slug_of(text: &str) -> &str {
    let mut lines = text.lines();
    lines.by_ref().find(|l| l.trim() == "PII-SKILL");
    lines.map(str::trim).find(|l| !l.is_empty()).unwrap_or("-")
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
