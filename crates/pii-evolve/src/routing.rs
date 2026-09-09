//! Auditable per-step model routing (R4).
//!
//! An explicit step -> tier -> model table, resolved by
//! [`RoutePolicy::resolve`], plus an append-only [`RouteLog`] so any
//! misroute can be traced back to the decision that produced it. Model
//! diversity — `review` configured to a different model than `patch` — is
//! a policy fact this module records, never infers.

use std::collections::BTreeMap;

use pii_core::{Ctx, PiiError};

/// The model-routing tiers a boss step maps onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Fast, cheap scouting: locate the file/lines a signal points at.
    Localize,
    /// A strong worker model that writes the patch.
    Patch,
    /// A different strong model reviewing the patch (model diversity).
    Review,
    /// A planner-tier model for periodic research.
    Research,
    /// The boss itself, coordinating the cycle.
    Orchestrator,
}

/// One routing decision: `step` resolved to `tier`, which resolved to
/// `model`, at time `ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    /// The step name that was routed.
    pub step: String,
    /// The tier the step's policy entry maps to.
    pub tier: Tier,
    /// The model spec configured for that tier.
    pub model: String,
    /// Milliseconds since the epoch when the decision was made.
    pub ts: u64,
}

/// The explicit step -> tier -> model table.
#[derive(Debug, Clone, Default)]
pub struct RoutePolicy {
    step_tiers: BTreeMap<String, Tier>,
    models: BTreeMap<Tier, String>,
}

impl RoutePolicy {
    /// An empty policy: no steps or models configured.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns `step` to `tier`.
    pub fn assign_step(&mut self, step: impl Into<String>, tier: Tier) -> &mut Self {
        self.step_tiers.insert(step.into(), tier);
        self
    }

    /// Configures the model spec used for `tier`.
    pub fn set_model(&mut self, tier: Tier, model: impl Into<String>) -> &mut Self {
        self.models.insert(tier, model.into());
        self
    }

    /// The model configured for `tier`, if any.
    #[must_use]
    pub fn model_for(&self, tier: Tier) -> Option<&str> {
        self.models.get(&tier).map(String::as_str)
    }

    /// Resolves `step` to its configured tier and model.
    ///
    /// # Errors
    /// [`PiiError::NotFound`] when `step` has no tier assignment, or the
    /// assigned tier has no configured model.
    pub fn resolve(&self, step: &str, ctx: &Ctx) -> Result<RouteDecision, PiiError> {
        let tier = self
            .step_tiers
            .get(step)
            .copied()
            .ok_or_else(|| PiiError::NotFound(format!("no tier assigned for step {step:?}")))?;
        let model = self
            .model_for(tier)
            .ok_or_else(|| PiiError::NotFound(format!("no model configured for tier {tier:?}")))?
            .to_owned();
        Ok(RouteDecision {
            step: step.to_owned(),
            tier,
            model,
            ts: ctx.now_ms(),
        })
    }
}

/// An append-only, auditable record of every routing decision made during a
/// cycle.
#[derive(Debug, Clone, Default)]
pub struct RouteLog {
    entries: Vec<RouteDecision>,
}

impl RouteLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves `step` against `policy` and appends the decision to the
    /// log.
    ///
    /// # Errors
    /// Propagates [`RoutePolicy::resolve`]'s errors; nothing is appended on
    /// failure.
    pub fn route(
        &mut self,
        policy: &RoutePolicy,
        step: &str,
        ctx: &Ctx,
    ) -> Result<RouteDecision, PiiError> {
        let decision = policy.resolve(step, ctx)?;
        self.entries.push(decision.clone());
        Ok(decision)
    }

    /// Every decision recorded so far, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[RouteDecision] {
        &self.entries
    }
}
