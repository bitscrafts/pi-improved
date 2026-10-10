//! Model tiers (spec 114): the `[[model_tier]]` table, the one model-
//! precedence function, and the per-tier health report.
//!
//! A tier names a provider spec, optionally an allowlist of tools and a
//! `max_tokens` limit. The tool list is enforced by the capability layer
//! (`Ctx::with_tools` / `Ctx::tool`), never filtered after the fact.

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_abi::PiiError;
use pii_model::ModelProvider;
use pii_observe::{Health, State, SubsystemHealth};

use crate::daemon::config::ModelTierSection;

/// The level that supplied a model, highest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// A node's explicit `model` (or its `route_step` policy entry).
    NodeModel,
    /// A node's `tier`.
    NodeTier,
    /// The run's model (`--model`).
    RunModel,
    /// `$PII_MODEL`.
    Env,
    /// The daemon config's `[core] model`.
    Config,
}

/// THE model precedence (spec 114 R2), the only place it is written down.
///
/// Node `model` > node `tier` > run model > `$PII_MODEL` > `[core] model`
/// (env over config is the pre-existing, documented order; it is kept.)
#[must_use]
pub fn precedence(
    node_model: Option<String>,
    node_tier: Option<String>,
    run_model: Option<String>,
    env: Option<String>,
    config: Option<String>,
) -> Option<(Level, String)> {
    [
        (Level::NodeModel, node_model),
        (Level::NodeTier, node_tier),
        (Level::RunModel, run_model),
        (Level::Env, env),
        (Level::Config, config),
    ]
    .into_iter()
    .find_map(|(l, m)| m.map(|m| (l, m)))
}

/// One configured tier and the provider built for it (or why it has none).
pub struct Tier {
    /// Tier name.
    pub name: String,
    /// Provider spec.
    pub model: String,
    /// Tool allowlist; `None` = unrestricted.
    pub tools: Option<Vec<String>>,
    /// Declared `max_tokens` limit.
    pub max_tokens: Option<u32>,
    provider: Result<Arc<dyn ModelProvider>, String>,
}

impl Tier {
    /// The tier's provider.
    ///
    /// # Errors
    /// [`PiiError::Unavailable`] naming the tier and the provider spec when the
    /// provider could not be built. There is no fallback to another tier.
    pub fn provider(&self) -> Result<Arc<dyn ModelProvider>, PiiError> {
        self.provider.clone().map_err(|why| {
            PiiError::Unavailable(format!(
                "model tier '{}': provider '{}' unavailable: {why}",
                self.name, self.model
            ))
        })
    }
}

impl Health for Tier {
    fn health(&self) -> SubsystemHealth {
        let mut h = SubsystemHealth::ok(format!("model_tier:{}", self.name));
        h.identity.insert("model".into(), self.model.clone());
        if let Some(t) = &self.tools {
            h.identity.insert("tools".into(), t.join(","));
        }
        if let Err(why) = &self.provider {
            h.state = State::Degraded;
            h.detail = Some(format!("provider '{}' unavailable: {why}", self.model));
        }
        h
    }
}

/// Every configured tier, by name.
#[derive(Default)]
pub struct TierTable {
    tiers: BTreeMap<String, Tier>,
}

impl TierTable {
    /// Builds the table, constructing each tier's provider with `make`. A
    /// provider that fails to build marks only ITS tier unavailable and is
    /// reported through `on_unavailable`; the other tiers are unaffected.
    pub fn build(
        sections: &[ModelTierSection],
        make: impl Fn(&str) -> Result<Arc<dyn ModelProvider>, PiiError>,
        on_unavailable: impl Fn(&str, &str, &str),
    ) -> Self {
        let tiers = sections
            .iter()
            .map(|s| {
                let provider = make(&s.model).map_err(|e| e.to_string());
                if let Err(why) = &provider {
                    on_unavailable(&s.name, &s.model, why);
                }
                let tier = Tier {
                    name: s.name.clone(),
                    model: s.model.clone(),
                    tools: s.tools.clone(),
                    max_tokens: s.max_tokens,
                    provider,
                };
                (s.name.clone(), tier)
            })
            .collect();
        Self { tiers }
    }

    /// Looks a tier up.
    ///
    /// # Errors
    /// [`PiiError::NotFound`] naming the tier and the available ones.
    pub fn get(&self, name: &str) -> Result<&Tier, PiiError> {
        self.tiers.get(name).ok_or_else(|| {
            let avail: Vec<&str> = self.tiers.keys().map(String::as_str).collect();
            PiiError::NotFound(format!(
                "model tier '{name}' is not configured (available: {})",
                if avail.is_empty() {
                    String::from("none")
                } else {
                    avail.join(", ")
                }
            ))
        })
    }

    /// Every tier, as health reporters.
    pub fn health_members(&self) -> impl Iterator<Item = &dyn Health> {
        self.tiers.values().map(|t| t as &dyn Health)
    }
}
