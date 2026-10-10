//! The enforcing capability context.
//!
//! [`Ctx`] combines a [`CancelToken`], a [`Budget`], a [`CapSet`], a
//! [`Principal`], a [`TraceCtx`], and an observer. Accessors return `Result`;
//! the check happens here, not in a doc comment. `Ctx::child` is monotonic
//! attenuation by construction: capabilities are intersected, budget is
//! carved (never duplicated), cancel is a child token, and span increments.

use std::sync::Arc;
use std::time::Instant;

use pii_abi::{CapKind, PiiError};

use crate::budget::{Budget, BudgetShare};
use crate::cancel::CancelToken;
use crate::cap::CapSet;
use crate::childproc::ProcCap;
use crate::clock::Clock;
use crate::fscap::FsCap;
use crate::observe::{NullObserver, Observer};
use crate::principal::{Principal, TraceCtx};

struct CtxInner {
    cancel: CancelToken,
    started: Instant,
    deadline: Option<Instant>,
    deadline_ms: Option<u64>,
    clock: Arc<dyn Clock>,
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
                deadline_ms: None,
                clock: Arc::new(crate::clock::SystemClock),
                budget: Arc::new(budget),
                caps: Arc::new(caps),
                principal: Principal::User,
                trace,
                obs,
            }),
        }
    }

    /// A root context with an observer, trace context, and an explicit
    /// cancel token (F5.3).
    ///
    /// Callers that need to trip cancellation from outside the context tree
    /// (a run registry keeping the token to cancel later) must supply that
    /// SAME token here — [`Ctx::with_observer`] mints a fresh, unreachable
    /// one, so a token registered elsewhere would never actually reach the
    /// running node.
    #[must_use]
    pub fn with_observer_and_cancel(
        caps: CapSet,
        budget: Budget,
        obs: Arc<dyn Observer>,
        trace: TraceCtx,
        cancel: CancelToken,
    ) -> Self {
        Self {
            inner: Arc::new(CtxInner {
                cancel,
                started: Instant::now(),
                deadline: None,
                deadline_ms: None,
                clock: Arc::new(crate::clock::SystemClock),
                budget: Arc::new(budget),
                caps: Arc::new(caps),
                principal: Principal::User,
                trace,
                obs,
            }),
        }
    }

    /// Replaces the clock (F1.3): used by tests and by the daemon, which
    /// installs the process-wide clock once at startup.
    #[must_use]
    pub fn with_clock(self, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(CtxInner {
                cancel: self.inner.cancel.clone(),
                started: self.inner.started,
                deadline: self.inner.deadline,
                deadline_ms: self.inner.deadline_ms,
                clock,
                budget: Arc::clone(&self.inner.budget),
                caps: Arc::clone(&self.inner.caps),
                principal: self.inner.principal.clone(),
                trace: self.inner.trace.clone(),
                obs: Arc::clone(&self.inner.obs),
            }),
        }
    }

    /// Sets an absolute deadline `ms_from_now` milliseconds from the
    /// context's clock (F1.6). One deadline spans all stages of a run.
    #[must_use]
    pub fn with_deadline(self, ms_from_now: u64) -> Self {
        let now = self.inner.clock.now_ms();
        let at = now.saturating_add(ms_from_now);
        let remaining = std::time::Duration::from_millis(ms_from_now);
        let deadline = Some(self.inner.started + remaining);
        Self {
            inner: Arc::new(CtxInner {
                cancel: self.inner.cancel.clone(),
                started: self.inner.started,
                deadline,
                deadline_ms: Some(at),
                clock: Arc::clone(&self.inner.clock),
                budget: Arc::clone(&self.inner.budget),
                caps: Arc::clone(&self.inner.caps),
                principal: self.inner.principal.clone(),
                trace: self.inner.trace.clone(),
                obs: Arc::clone(&self.inner.obs),
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
        if caps.intersects(CapKind::FS_READ | CapKind::FS_WRITE) {
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
        if self.inner.caps.kinds.contains(CapKind::NET) {
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
        if self.inner.caps.kinds.contains(CapKind::PROC) {
            Ok(ProcCap { ctx: self })
        } else {
            Err(self.deny("proc"))
        }
    }

    /// The tool gate (spec 114): a context whose `tools` allowlist is set
    /// refuses any tool outside it, with a denial event.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] (`tool:<name>`) when the allowlist is set
    /// and does not contain `name`.
    pub fn tool(&self, name: &str) -> Result<(), PiiError> {
        match &self.inner.caps.tools {
            Some(allowed) if !allowed.iter().any(|t| t == name) => {
                Err(self.deny(&format!("tool:{name}")))
            }
            _ => Ok(()),
        }
    }

    /// Attenuation by tool name: the same context with its tool allowlist
    /// intersected with `names`. Never widens; every other capability, the
    /// budget, cancel token and principal are shared unchanged.
    #[must_use]
    pub fn with_tools(&self, names: &[String]) -> Self {
        let narrow = CapSet {
            tools: Some(names.to_vec()),
            ..self.inner.caps.as_ref().clone()
        };
        Self {
            inner: Arc::new(CtxInner {
                cancel: self.inner.cancel.clone(),
                started: self.inner.started,
                deadline: self.inner.deadline,
                deadline_ms: self.inner.deadline_ms,
                clock: Arc::clone(&self.inner.clock),
                budget: Arc::clone(&self.inner.budget),
                caps: Arc::new(self.inner.caps.intersect(&narrow)),
                principal: self.inner.principal.clone(),
                trace: self.inner.trace.clone(),
                obs: Arc::clone(&self.inner.obs),
            }),
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
        let deadline_ms = self.inner.deadline_ms;
        let clock = Arc::clone(&self.inner.clock);
        let obs = Arc::clone(&self.inner.obs);
        Self {
            inner: Arc::new(CtxInner {
                cancel: token,
                started,
                deadline,
                deadline_ms,
                clock,
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

    /// Current wall-clock time in ms since the epoch, via the injected
    /// clock (F1.3).
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    /// The absolute deadline in epoch-ms, when set (F1.6).
    #[must_use]
    pub fn deadline_ms(&self) -> Option<u64> {
        self.inner.deadline_ms
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

    /// The principal acting through this context (F5.6: node kinds that need
    /// a run/node-scoped resume key read it from here rather than parsing
    /// [`Ctx::principal_str`]).
    #[must_use]
    pub fn principal(&self) -> &Principal {
        &self.inner.principal
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

    pub(crate) fn deny(&self, cap: &str) -> PiiError {
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
            fs_write: PathScope::from_strings(&[Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/_tmp"
            ))]),
            fs_read: PathScope::from_strings(&[Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/_tmp"
            ))]),
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
    fn ctx_tool_gate_denies_outside_scope_with_event() {
        let obs = Arc::new(RecordingObserver::new());
        let base = Ctx::with_observer(
            caps(CapKind::FS_READ),
            Budget::unlimited(),
            obs.clone(),
            TraceCtx {
                run: pii_abi::RunId::from("t"),
                span: 0,
            },
        );
        // Unrestricted until a tier narrows it.
        assert!(base.tool("bash").is_ok());
        let tiered = base.with_tools(&[String::from("fs_read")]);
        assert!(tiered.tool("fs_read").is_ok(), "inside the list is allowed");
        let err = tiered.tool("bash").unwrap_err();
        assert!(matches!(err, PiiError::Denied(_)), "got {err:?}");
        assert!(
            obs.events().iter().any(|e| matches!(
                e,
                crate::observe::ObsEvent::Denial { cap, .. } if cap == "tool:bash"
            )),
            "a denial event names the tool"
        );
        // Attenuation is monotonic: a second narrowing can never re-widen.
        let narrower = tiered.with_tools(&[String::from("fs_read"), String::from("bash")]);
        assert!(narrower.tool("bash").is_err(), "cannot widen");
        assert!(narrower.tool("fs_read").is_ok());
        // The parent is untouched.
        assert!(base.tool("bash").is_ok());
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
            .write(
                Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/x")),
                b"x",
            )
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
