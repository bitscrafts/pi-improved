//! The enforcing capability context.
//!
//! [`Ctx`] combines a [`CancelToken`], a [`Budget`], a [`CapSet`], a
//! [`Principal`], a [`TraceCtx`], and an observer. Accessors return `Result`;
//! the check happens here, not in a doc comment. `Ctx::child` is monotonic
//! attenuation by construction: capabilities are intersected, budget is
//! carved (never duplicated), cancel is a child token, and span increments.

use std::sync::Arc;
use std::time::Instant;

use pii_abi::PiiError;

use crate::budget::{Budget, BudgetShare};
use crate::cancel::CancelToken;
use crate::cap::CapSet;
use crate::fscap::FsCap;
use crate::observe::{NullObserver, Observer};
use crate::principal::{Principal, TraceCtx};

struct CtxInner {
    cancel: CancelToken,
    started: Instant,
    deadline: Option<Instant>,
    budget: Arc<Budget>,
    caps: Arc<CapSet>,
    principal: Principal,
    trace: TraceCtx,
    obs: Arc<dyn Observer>,
}

/// An enforcing, cloneable, `Send + Sync` capability context.
#[derive(Clone)]
pub struct Ctx {
    inner: Arc<CtxInner>,
}

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx")
            .field("principal", &self.inner.principal)
            .field("caps", &self.inner.caps.kinds)
            .field("span", &self.inner.trace.span)
            .finish_non_exhaustive()
    }
}

impl Ctx {
    /// A root context with no attenuation and a null observer.
    #[must_use]
    pub fn root(caps: CapSet, budget: Budget) -> Self {
        Self::with_observer(
            caps,
            budget,
            Arc::new(NullObserver),
            TraceCtx {
                run: pii_abi::RunId::from("root"),
                span: 0,
            },
        )
    }

    /// A root context with an observer and trace context.
    #[must_use]
    pub fn with_observer(
        caps: CapSet,
        budget: Budget,
        obs: Arc<dyn Observer>,
        trace: TraceCtx,
    ) -> Self {
        Self {
            inner: Arc::new(CtxInner {
                cancel: CancelToken::new(),
                started: Instant::now(),
                deadline: None,
                budget: Arc::new(budget),
                caps: Arc::new(caps),
                principal: Principal::User,
                trace,
                obs,
            }),
        }
    }

    /// The filesystem capability gate.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] unless the context grants `FS_READ` or
    /// `FS_WRITE`.
    pub fn fs(&self) -> Result<FsCap<'_>, PiiError> {
        let caps = &self.inner.caps.kinds;
        if caps.intersects(crate::CapKind::FS_READ | crate::CapKind::FS_WRITE) {
            Ok(FsCap::new(self))
        } else {
            Err(self.deny("fs"))
        }
    }

    /// The network capability gate. Phase 1: no methods.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] unless the context grants `NET`.
    pub fn net(&self) -> Result<NetCap<'_>, PiiError> {
        if self.inner.caps.kinds.contains(crate::CapKind::NET) {
            Ok(NetCap { ctx: self })
        } else {
            Err(self.deny("net"))
        }
    }

    /// The process capability gate. Phase 1: no methods.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] unless the context grants `PROC`.
    pub fn proc(&self) -> Result<ProcCap<'_>, PiiError> {
        if self.inner.caps.kinds.contains(crate::CapKind::PROC) {
            Ok(ProcCap { ctx: self })
        } else {
            Err(self.deny("proc"))
        }
    }

    /// Attenuation only: caps = self ∩ narrow; budget CARVED from self (never
    /// duplicated); cancel is a child token; principal is replaced; span
    /// increments.
    #[must_use]
    pub fn child(&self, principal: Principal, narrow: &CapSet, share: &BudgetShare) -> Self {
        let cancel = self.inner.cancel.child();
        self.child_with_token(principal, narrow, share, cancel)
    }

    /// Like [`Ctx::child`], but installs `token` as this context's own cancel
    /// token (which may still observe its ancestors). The scheduler uses this
    /// so a per-node token registered in a side table is the *same* token the
    /// node observes — `cancel_losers` must reach the running thread.
    #[must_use]
    pub fn child_with_token(
        &self,
        principal: Principal,
        narrow: &CapSet,
        share: &BudgetShare,
        token: CancelToken,
    ) -> Self {
        let caps = Arc::new(self.inner.caps.intersect(narrow));
        // `Shared` shares the parent's budget Arc (no carve); every other
        // share carves out of the parent (never duplicated). Same-process
        // nodes share one budget; carving is for subgraph/worker boundaries.
        let budget = match share {
            BudgetShare::Shared => Arc::clone(&self.inner.budget),
            _ => Arc::new(self.inner.budget.carve(share)),
        };
        let trace = TraceCtx {
            run: self.inner.trace.run.clone(),
            span: self.inner.trace.span + 1,
        };
        let started = self.inner.started;
        let deadline = self.inner.deadline;
        let obs = Arc::clone(&self.inner.obs);
        Self {
            inner: Arc::new(CtxInner {
                cancel: token,
                started,
                deadline,
                budget,
                caps,
                principal,
                trace,
                obs,
            }),
        }
    }

    /// True if this context (or an ancestor) is cancelled.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }

    /// Milliseconds elapsed since the root context was created.
    #[must_use]
    pub fn elapsed_ms(&self) -> u32 {
        u32::try_from(self.inner.started.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    /// Checks for cancellation then deadline.
    ///
    /// # Errors
    /// Returns [`PiiError::Cancelled`] if cancelled, or [`PiiError::Timeout`]
    /// if the deadline has passed.
    pub fn check(&self) -> Result<(), PiiError> {
        if self.cancelled() {
            return Err(PiiError::Cancelled);
        }
        if let Some(deadline) = self.inner.deadline {
            let now = Instant::now();
            if now >= deadline {
                let after_ms =
                    u64::try_from(deadline.duration_since(self.inner.started).as_millis())
                        .unwrap_or(u64::MAX);
                return Err(PiiError::Timeout { after_ms });
            }
        }
        Ok(())
    }

    /// The observer.
    #[must_use]
    pub fn observer(&self) -> &Arc<dyn Observer> {
        &self.inner.obs
    }

    /// The effective (attenuated) capability set.
    #[must_use]
    pub fn caps(&self) -> &CapSet {
        &self.inner.caps
    }

    /// The budget.
    #[must_use]
    pub fn budget(&self) -> &Budget {
        &self.inner.budget
    }

    /// The principal as a display string.
    #[must_use]
    pub fn principal_str(&self) -> String {
        self.inner.principal.to_string()
    }

    /// The trace context.
    #[must_use]
    pub fn trace(&self) -> TraceCtx {
        self.inner.trace.clone()
    }

    /// The cancel token.
    #[must_use]
    pub fn cancel_token(&self) -> &CancelToken {
        &self.inner.cancel
    }

    fn deny(&self, cap: &str) -> PiiError {
        self.observer().event(&crate::observe::ObsEvent::Denial {
            principal: self.principal_str(),
            cap: String::from(cap),
        });
        PiiError::Denied(cap.to_string())
    }
}

/// Net gate. Phase 1 holds no methods.
#[derive(Debug)]
pub struct NetCap<'a> {
    #[allow(unused)]
    ctx: &'a Ctx,
}

/// Process gate. Phase 1 holds no methods.
#[derive(Debug)]
pub struct ProcCap<'a> {
    #[allow(unused)]
    ctx: &'a Ctx,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::path::Path;
    use std::sync::Arc;

    use pii_abi::PiiError;

    use super::Ctx;
    use crate::CapKind;
    use crate::budget::{Budget, BudgetShare};
    use crate::cap::{CapSet, PathScope};
    use crate::observe::RecordingObserver;
    use crate::principal::{Principal, TraceCtx};

    fn caps(kinds: CapKind) -> CapSet {
        CapSet {
            kinds,
            fs_write: PathScope::from_strings(&[Path::new("/projects/pi-improved/_tmp")]),
            fs_read: PathScope::from_strings(&[Path::new("/projects/pi-improved/_tmp")]),
            ..CapSet::default()
        }
    }

    fn root(kinds: CapKind) -> Ctx {
        Ctx::with_observer(
            caps(kinds),
            Budget::unlimited(),
            Arc::new(RecordingObserver::new()),
            TraceCtx {
                run: pii_abi::RunId::from("t"),
                span: 0,
            },
        )
    }

    #[test]
    fn ctx_child_intersects_caps() {
        let root = root(CapKind::FS_READ | CapKind::FS_WRITE);
        let narrow = caps(CapKind::FS_READ);
        let child = root.child(Principal::User, &narrow, &BudgetShare::All);
        assert_eq!(child.caps().kinds, CapKind::FS_READ);
        assert!(child.fs().is_ok());
        let err = child
            .fs()
            .unwrap()
            .write(Path::new("/projects/pi-improved/_tmp/x"), b"x")
            .unwrap_err();
        assert!(matches!(err, PiiError::Denied(c) if c == "fs_write"));
    }

    #[test]
    fn ctx_child_cannot_widen_caps() {
        let root = root(CapKind::FS_READ);
        let narrow = caps(CapKind::FS_READ | CapKind::NET);
        let child = root.child(Principal::User, &narrow, &BudgetShare::All);
        assert_eq!(child.caps().kinds, CapKind::FS_READ);
        let err = child.net().unwrap_err();
        assert!(matches!(err, PiiError::Denied(c) if c == "net"));
    }

    #[test]
    fn ctx_child_carves_budget() {
        let _root = root(CapKind::FS_READ);
        // Replace the unlimited budget with a token budget for this test.
        let b = Budget::with_tokens(1000);
        let single = Ctx::with_observer(
            caps(CapKind::FS_READ),
            b,
            Arc::new(RecordingObserver::new()),
            TraceCtx {
                run: pii_abi::RunId::from("t"),
                span: 0,
            },
        );
        let child = single.child(
            Principal::User,
            &caps(CapKind::FS_READ),
            &BudgetShare::Fraction(25),
        );
        assert_eq!(
            child.budget().remaining(crate::budget::BudgetKind::Tokens),
            250
        );
        assert_eq!(
            single.budget().remaining(crate::budget::BudgetKind::Tokens),
            750
        );
    }

    #[test]
    fn ctx_child_shared_budget_does_not_starve_siblings() {
        // F2 (opus review): same-process nodes share one budget; a `Shared`
        // child must not drain the parent so a later sibling still has budget.
        let b = Budget::with_tokens(1000);
        let root = Ctx::with_observer(
            caps(CapKind::FS_READ),
            b,
            Arc::new(RecordingObserver::new()),
            TraceCtx {
                run: pii_abi::RunId::from("t"),
                span: 0,
            },
        );
        let a = root.child(
            Principal::User,
            &caps(CapKind::FS_READ),
            &BudgetShare::Shared,
        );
        let b2 = root.child(
            Principal::User,
            &caps(CapKind::FS_READ),
            &BudgetShare::Shared,
        );
        // Both children see the same shared budget (1000), not a drained one.
        assert_eq!(
            a.budget().remaining(crate::budget::BudgetKind::Tokens),
            1000
        );
        assert_eq!(
            b2.budget().remaining(crate::budget::BudgetKind::Tokens),
            1000
        );
        // A charge by one is visible to the other (they share the Arc).
        a.budget()
            .charge(crate::budget::BudgetKind::Tokens, 300)
            .unwrap();
        assert_eq!(
            b2.budget().remaining(crate::budget::BudgetKind::Tokens),
            700
        );
    }
}
