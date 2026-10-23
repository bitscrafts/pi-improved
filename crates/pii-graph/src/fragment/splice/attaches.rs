//! Parsing of a graph document's `[[attach]]` tables into [`Attach`] records (split from `splice.rs` for the 450-line gate).

use pii_abi::PiiError;
use toml::Table;

use crate::fragment::check::Attach;
use crate::fragment::shared::SURFACES;

/// The `[[attach]]` tables of a host graph document, validated (id, surface, target names, pin, fragment).
pub(in crate::fragment) fn parse_attaches(doc: &Table) -> Result<Vec<Attach>, PiiError> {
    let items = match doc.get("attach") {
        None => return Ok(Vec::new()),
        Some(toml::Value::Array(items)) => items,
        Some(_) => {
            return Err(PiiError::Invalid(
                "'attach' must be an array of tables".into(),
            ));
        }
    };
    let mut out: Vec<Attach> = Vec::new();
    for item in items {
        let t = item
            .as_table()
            .ok_or_else(|| PiiError::Invalid("each [[attach]] must be a table".into()))?;
        let id = text(t, "id")?;
        if out.iter().any(|a| a.id == id) {
            return Err(PiiError::Invalid(format!("duplicate attach id '{id}'")));
        }
        let at = text(t, "at")?;
        if !SURFACES.contains(&at.as_str()) {
            return Err(PiiError::Invalid(format!(
                "fragment.surface.unknown: attach '{id}' names surface '{at}'"
            )));
        }
        let on = names(t, &id)?;
        if at.starts_with("node.") && on.is_empty() {
            return Err(PiiError::Invalid(format!(
                "attach '{id}' at {at} needs on = [node ids] or [\"*\"]"
            )));
        }
        if !at.starts_with("node.") && !on.is_empty() {
            return Err(PiiError::Invalid(format!(
                "attach '{id}' at {at} takes no on list"
            )));
        }
        let pin = text(t, "pin").map_err(|_| {
            PiiError::Invalid(format!(
                "fragment.pin.missing: attach '{id}' carries no pin; a fragment is inlined only by its sha256"
            ))
        })?;
        let order = match t.get("order") {
            None => 0,
            Some(v) => v.as_integer().ok_or_else(|| {
                PiiError::Invalid(format!("attach '{id}' order must be an integer"))
            })?,
        };
        out.push(Attach {
            fragment: text(t, "fragment")?,
            id,
            at,
            on,
            order,
            pin: pin.to_ascii_lowercase(),
        });
    }
    Ok(out)
}

fn text(t: &Table, key: &str) -> Result<String, PiiError> {
    t.get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PiiError::Invalid(format!("attach missing string '{key}'")))
}

fn names(t: &Table, id: &str) -> Result<Vec<String>, PiiError> {
    match t.get("on") {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    PiiError::Invalid(format!("attach '{id}': 'on' entries must be strings"))
                })
            })
            .collect(),
        Some(_) => Err(PiiError::Invalid(format!(
            "attach '{id}': 'on' must be an array"
        ))),
    }
}
