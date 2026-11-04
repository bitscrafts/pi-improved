//! The unattended learn timer (spec 139 R1): every `[core].learn_interval_secs`
//! (daily by default) one extraction + verification pass. The first pass is one
//! period after start, never at start. The pass cannot promote: it has no path
//! to the promoter, so promotion stays a human act.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::Engine;

/// How often the daemon advances the promotion lifecycle (spec 125).
const PROMOTION_SWEEP_SECS: u64 = 5;

/// Spec 125 R4/R5: every [`PROMOTION_SWEEP_SECS`] lapse promotions past their
/// TTL and evaluate those whose window has closed (reverting any that did not
/// meet their metric). The blocking work runs off the runtime.
pub(super) fn spawn_promotion(engine: Arc<Engine>, dir: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(PROMOTION_SWEEP_SECS));
        // Spec 371 R2: what the last sweep saw, so an unchanged directory is not re-parsed every tick.
        let gate = Arc::new(parking_lot::Mutex::new(pii_learn::SweepGate::default()));
        loop {
            tick.tick().await;
            let (e, d, g) = (Arc::clone(&engine), dir.clone(), Arc::clone(&gate));
            let _ = tokio::task::spawn_blocking(move || {
                crate::engine::learn_methods::promotion_sweep_gated(&e, &d, &mut g.lock())
            })
            .await;
        }
    })
}

/// Spawns the timer; the blocking pass runs off the runtime.
pub(super) fn spawn(
    engine: Arc<Engine>,
    dir: PathBuf,
    period: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            tick.tick().await;
            let (e, d) = (Arc::clone(&engine), dir.clone());
            let _ = tokio::task::spawn_blocking(move || {
                crate::engine::learn_methods::learn_unattended_pass(&e, &d)
            })
            .await;
        }
    })
}
