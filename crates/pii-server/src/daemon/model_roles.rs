//! Model roles (spec 362): the one place a model id is named. A `[models.<role>]` table maps a role to a model and a
//! provider, and a node says `model = "@<role>"` to take both. Split out of `daemon/config.rs` to stay under the
//! 450-line file budget (spec 362 decision, 2026-11-04).

use std::collections::BTreeMap;

use pii_abi::PiiError;
use serde::Deserialize;

/// `[models.<role>]` (spec 362).
///
/// ```toml
/// [models.workhorse]
/// model = "deepseek-v4.1-flash"
/// provider = "ollama-cloud"
/// ```
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelRole {
    /// Model id handed to the agent (`--model`).
    pub model: String,
    /// Provider the agent must use (`--provider`).
    pub provider: String,
}

/// An empty role table: the value when a config names no `[models]` (spec 362).
#[must_use]
pub fn empty_model_roles() -> BTreeMap<String, ModelRole> {
    BTreeMap::new()
}

/// Refuses a `[models.<role>]` entry whose model the provider does not list (spec 362 R4).
///
/// Names the entry and the id. The caller supplies the list; an unreachable provider is the caller's to report and
/// skip.
///
/// # Errors
/// [`PiiError::Invalid`] naming the first role whose model is not in `listed`.
pub fn check_listed(
    roles: &BTreeMap<String, ModelRole>,
    listed: &[String],
) -> Result<(), PiiError> {
    for (name, role) in roles {
        if !listed.contains(&role.model) {
            return Err(PiiError::Invalid(format!(
                "config: [models.{name}] model '{}' is not on the provider's list",
                role.model
            )));
        }
    }
    Ok(())
}

/// Resolves a node's `model = "@<role>"` to the configured model and provider (spec 362 R2).
///
/// An explicit model id is left exactly as written. An unknown role is refused by name, listing the configured
/// roles; it is never defaulted (R3).
///
/// # Errors
/// [`PiiError::Invalid`] when the role is not in `roles`.
pub fn resolve_role(
    cfg: &mut serde_json::Value,
    roles: &BTreeMap<String, ModelRole>,
) -> Result<(), PiiError> {
    let Some(role) = cfg
        .get("model")
        .and_then(serde_json::Value::as_str)
        .and_then(|m| m.strip_prefix('@'))
        .map(str::to_owned)
    else {
        return Ok(());
    };
    let Some(found) = roles.get(&role) else {
        let known: Vec<&str> = roles.keys().map(String::as_str).collect();
        let listed = if known.is_empty() {
            String::from("none")
        } else {
            known.join(", ")
        };
        return Err(PiiError::Invalid(format!(
            "model role '@{role}' is not configured (configured roles: {listed})"
        )));
    };
    cfg["model"] = serde_json::Value::String(found.model.clone());
    cfg["provider"] = serde_json::Value::String(found.provider.clone());
    Ok(())
}
