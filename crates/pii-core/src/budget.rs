//! Carveable, never-duplicated accounting.

use std::sync::atomic::{AtomicU64, Ordering};

use pii_abi::PiiError;

/// Kinds of budget that can be charged and queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetKind {
    /// Token budget.
    Tokens,
    /// Tool-call budget.
    ToolCalls,
    /// Micro-dollar budget.
    MicroUsd,
    /// Wall-clock milliseconds budget.
    WallMs,
}

impl BudgetKind {
    /// The stable name used in errors.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tokens => "Tokens",
            Self::ToolCalls => "ToolCalls",
            Self::MicroUsd => "MicroUsd",
            Self::WallMs => "WallMs",
        }
    }
}

/// How a child budget is carved out of its parent.
#[derive(Debug, Clone)]
pub enum BudgetShare {
    /// Carve everything that remains.
    All,
    /// Carve this percentage (0..=100) of what remains.
    Fraction(u32),
    /// Carve exact amounts (clamped to what remains).
    Absolute {
        /// Token count.
        tokens: u64,
        /// Tool-call count.
        tool_calls: u64,
        /// Micro-dollar amount.
        micro_usd: u64,
        /// Wall-clock milliseconds.
        wall_ms: u64,
    },
    /// Share the parent's budget without carving (same-process nodes).
    ///
    /// Same-process nodes in one run draw from one shared budget; carving is
    /// reserved for `subgraph`/`worker` boundaries (pass-1 §3.3) where a child
    /// must be isolated. Using `All` for every node would drain the parent on
    /// the first carve and starve every later node (opus review F2).
    Shared,
}

/// An atomic, carveable budget across four dimensions.
#[derive(Debug)]
pub struct Budget {
    tokens: AtomicU64,
    tool_calls: AtomicU64,
    micro_usd: AtomicU64,
    wall_ms: AtomicU64,
}

impl Budget {
    /// An effectively unbounded budget in every dimension.
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            tokens: AtomicU64::new(u64::MAX),
            tool_calls: AtomicU64::new(u64::MAX),
            micro_usd: AtomicU64::new(u64::MAX),
            wall_ms: AtomicU64::new(u64::MAX),
        }
    }

    /// A budget with explicit limits in every dimension.
    #[must_use]
    pub fn new(tokens: u64, tool_calls: u64, micro_usd: u64, wall_ms: u64) -> Self {
        Self {
            tokens: AtomicU64::new(tokens),
            tool_calls: AtomicU64::new(tool_calls),
            micro_usd: AtomicU64::new(micro_usd),
            wall_ms: AtomicU64::new(wall_ms),
        }
    }

    /// A budget with a fixed token count (all others unbounded).
    #[must_use]
    pub fn with_tokens(tokens: u64) -> Self {
        Self {
            tokens: AtomicU64::new(tokens),
            tool_calls: AtomicU64::new(u64::MAX),
            micro_usd: AtomicU64::new(u64::MAX),
            wall_ms: AtomicU64::new(u64::MAX),
        }
    }

    #[inline]
    fn field(&self, kind: BudgetKind) -> &AtomicU64 {
        match kind {
            BudgetKind::Tokens => &self.tokens,
            BudgetKind::ToolCalls => &self.tool_calls,
            BudgetKind::MicroUsd => &self.micro_usd,
            BudgetKind::WallMs => &self.wall_ms,
        }
    }

    /// Remaining budget for a kind.
    #[must_use]
    pub fn remaining(&self, kind: BudgetKind) -> u64 {
        self.field(kind).load(Ordering::SeqCst)
    }

    /// Charges `amount` against a kind, returning the new remaining amount.
    ///
    /// # Errors
    /// Returns [`PiiError::BudgetExhausted`] if `amount` exceeds what remains;
    /// the counter is left at 0.
    pub fn charge(&self, kind: BudgetKind, amount: u64) -> Result<u64, PiiError> {
        let field = self.field(kind);
        let mut cur = field.load(Ordering::SeqCst);
        loop {
            let next = cur.saturating_sub(amount);
            match field.compare_exchange_weak(cur, next, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => {
                    if cur < amount {
                        return Err(PiiError::BudgetExhausted(kind.name().to_string()));
                    }
                    return Ok(next);
                }
                Err(actual) => cur = actual,
            }
        }
    }

    /// Atomically removes the carved share from `self` (or clamps to what
    /// remains) and returns a new budget holding it. Never duplicates.
    #[must_use]
    pub fn carve(&self, share: &BudgetShare) -> Self {
        let (t, tc, m, w) = match share {
            // `Shared` is handled by `Ctx::child_with_token` (it clones the
            // parent's Arc and never calls `carve`). Defensive: treat it as
            // `All` if it ever reaches here.
            BudgetShare::All | BudgetShare::Shared => (
                self.remaining(BudgetKind::Tokens),
                self.remaining(BudgetKind::ToolCalls),
                self.remaining(BudgetKind::MicroUsd),
                self.remaining(BudgetKind::WallMs),
            ),
            BudgetShare::Fraction(pct) => {
                let pct = u64::from((*pct).min(100));
                (
                    self.remaining(BudgetKind::Tokens).saturating_mul(pct) / 100,
                    self.remaining(BudgetKind::ToolCalls).saturating_mul(pct) / 100,
                    self.remaining(BudgetKind::MicroUsd).saturating_mul(pct) / 100,
                    self.remaining(BudgetKind::WallMs).saturating_mul(pct) / 100,
                )
            }
            BudgetShare::Absolute {
                tokens,
                tool_calls,
                micro_usd,
                wall_ms,
            } => (*tokens, *tool_calls, *micro_usd, *wall_ms),
        };
        // carve clamps to what actually remains: debit = before - after.
        let take = |kind: BudgetKind, want: u64| -> u64 {
            let before = self.field(kind).load(Ordering::SeqCst);
            let _ = self.charge(kind, want);
            let after = self.field(kind).load(Ordering::SeqCst);
            before - after
        };
        let t = take(BudgetKind::Tokens, t);
        let tc = take(BudgetKind::ToolCalls, tc);
        let m = take(BudgetKind::MicroUsd, m);
        let w = take(BudgetKind::WallMs, w);
        Self {
            tokens: AtomicU64::new(t),
            tool_calls: AtomicU64::new(tc),
            micro_usd: AtomicU64::new(m),
            wall_ms: AtomicU64::new(w),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Budget, BudgetKind, BudgetShare};

    #[test]
    fn carve_fraction_removes_from_parent() {
        let parent = Budget::with_tokens(1000);
        let child = parent.carve(&BudgetShare::Fraction(25));
        assert_eq!(child.remaining(BudgetKind::Tokens), 250);
        assert_eq!(parent.remaining(BudgetKind::Tokens), 750);
    }

    #[test]
    fn budget_charge_beyond_remaining_is_exhausted() {
        let b = Budget::with_tokens(10);
        let err = b.charge(BudgetKind::Tokens, 11).unwrap_err();
        assert!(matches!(err, crate::PiiError::BudgetExhausted(s) if s == "Tokens"));
        assert_eq!(b.remaining(BudgetKind::Tokens), 0);
    }
}
