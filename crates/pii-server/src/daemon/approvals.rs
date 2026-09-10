//! Approval resolution glue (F5.9/F5.10/F5.11).

use std::sync::Arc;

use pii_abi::{PiiError, RunId};
use pii_proto::event::Event;
use pii_store::approval::{Approval, ResolveOutcome};

use crate::Engine;

/// Resolves an approval and, on the first resolve, continues the run
/// (F5.10). Resolving twice is idempotent; an unknown id is `NotFound`.
///
/// # Errors
/// [`PiiError::NotFound`] for an unknown id; store failures otherwise.
pub fn resolve(
    engine: &Arc<Engine>,
    id: &str,
    allow: bool,
    note: Option<&str>,
) -> Result<ResolveOutcome, PiiError> {
    let now_ms = pii_core::SystemClock.now_ms();
    let outcome = engine.approvals.resolve_approval(id, allow, note, now_ms)?;
    if outcome.resumed {
        let approval = engine
            .approvals
            .get_approval(id)?
            .ok_or_else(|| PiiError::NotFound(format!("approval '{id}'")))?;
        engine.bus.broadcast(Event::ApprovalResolved {
            approval_id: id.to_owned(),
            allow,
        });
        crate::runs::spawn_resume(
            Arc::clone(engine),
            RunId::from(approval.run_id.as_str()),
            None,
        )?;
    }
    Ok(outcome)
}

/// Creates the pending approval record and emits `approval_requested`
/// (F5.11), reacting to a `RunSuspended` whose journal reason is
/// `Approval` (called from the daemon's suspend-handling loop).
///
/// # Errors
/// Store failures.
pub fn request(
    engine: &Arc<Engine>,
    approval_id: &str,
    run_id: &str,
    node: &str,
    policy: &str,
    prompt: &str,
) -> Result<(), PiiError> {
    let now_ms = pii_core::SystemClock.now_ms();
    engine.approvals.put_approval(&Approval {
        approval_id: approval_id.to_owned(),
        run_id: run_id.to_owned(),
        node: node.to_owned(),
        policy: policy.to_owned(),
        prompt: prompt.to_owned(),
        requested_at_ms: now_ms,
        decision: None,
        note: None,
        decided_at_ms: 0,
    })?;
    engine.bus.broadcast(Event::ApprovalRequested {
        approval_id: approval_id.to_owned(),
        run_id: run_id.to_owned(),
        policy: policy.to_owned(),
        prompt: prompt.to_owned(),
    });
    Ok(())
}

use pii_core::Clock as _;
