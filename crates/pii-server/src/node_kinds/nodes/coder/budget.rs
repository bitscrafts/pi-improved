//! Spec 276: the coder node's side of the run's micro-dollar budget.
//!
//! A claude run reports what it spent as `total_cost_usd` on its `result` line, once the run has ended. The coder
//! charges that figure to the run's `max_micro_usd`, so a graph with `[limits]` bounds the node that spends the money,
//! and a coder that starts on an empty budget refuses before it spawns anything. A pi run reports no cost, so it
//! charges nothing and a budget does not bound it: a documented limit, not a guessed charge. The charge is claude's
//! own self-report, so the budget is not exact.

use pii_core::{BudgetKind, Ctx};
use pii_observe::Event;

use super::super::agent_claude::run_facts;

/// The refusal a coder gives on an empty run budget (spec 276 R1).
const EXHAUSTED: &str = "run budget exhausted: max_micro_usd";

/// The value `coder.exited` journals as `budget_exhausted`: the dimension, never a cost or an argv value.
pub const EXHAUSTED_DIMENSION: &str = "max_micro_usd";

/// `Some(reason)` when the run's micro-dollar budget is spent, so the coder refuses before it spawns anything.
#[must_use]
pub fn start_gate(ctx: &Ctx) -> Option<String> {
    (ctx.budget().remaining(BudgetKind::MicroUsd) == 0).then(|| EXHAUSTED.to_owned())
}

/// The claude spend cap for one run: the node's own cap, or the run's remaining micro-dollars when those are finite
/// and smaller. An unlimited budget keeps the node's cap unchanged, `None` included.
#[must_use]
#[allow(clippy::cast_precision_loss)] // reason: the remainder is a cap in USD, where f64 precision is far finer than a cent
pub fn cap_usd(ctx: &Ctx, node_cap: Option<f64>) -> Option<f64> {
    let remaining = ctx.budget().remaining(BudgetKind::MicroUsd);
    if remaining == u64::MAX {
        return node_cap;
    }
    let left = remaining as f64 / 1e6;
    Some(node_cap.map_or(left, |cap| cap.min(left)))
}

/// Names the dimension a run spent to zero on its `coder.exited` event, and only then: a run that left budget carries no
/// such field (spec 276 R3). The value is the dimension, never a cost.
#[must_use]
pub fn exited_field(event: Event, exhausted: bool) -> Event {
    if exhausted {
        event.field("budget_exhausted", EXHAUSTED_DIMENSION.to_owned())
    } else {
        event
    }
}

/// Charges the run's budget with the cost a claude run reported in `agent_output`.
///
/// Returns whether the charge spent the budget to zero. No reported cost, or one that is not a positive finite number,
/// charges nothing.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the amount is finite and positive here, so the cast only drops the fraction left by rounding"
)]
pub fn charge_run(ctx: &Ctx, agent_output: &str) -> bool {
    // An unlimited budget is the `u64::MAX` sentinel: charging it would make it finite and cap every later run.
    if ctx.budget().remaining(BudgetKind::MicroUsd) == u64::MAX {
        return false;
    }
    let micro = run_facts(agent_output)
        .cost_usd
        .map(|usd| (usd * 1e6).round())
        .filter(|micro| micro.is_finite() && *micro > 0.0);
    let Some(micro) = micro else {
        return false;
    };
    // `charge` leaves the counter at 0 when the amount is more than what remains, so the remainder says it is spent.
    let _ = ctx.budget().charge(BudgetKind::MicroUsd, micro as u64);
    ctx.budget().remaining(BudgetKind::MicroUsd) == 0
}
