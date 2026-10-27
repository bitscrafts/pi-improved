//! Scheduler state machine (spec F4.7-F4.9).
//!
//! Phase 1 so far ships the deterministic readiness/state layer. The tick
//! loop, dispatch with `cancel_losers`, and the blackboard land with the run
//! entry point in a dedicated pass.

pub mod barrier;
pub mod dispatch;
mod early_calls;
pub mod loop_helpers;
pub mod state;

pub use state::{
    EdgeState, Readiness, RunState, apply_completion, apply_failed_or_cancelled, propagate_dead,
    readiness,
};
