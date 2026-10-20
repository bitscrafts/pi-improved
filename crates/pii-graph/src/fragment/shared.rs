//! Rules both fragment checks and the weave share (spec 212).

/// Surfaces an attach may name.
pub(super) const SURFACES: &[&str] = &["run.start", "run.end", "node.before", "node.after"];
/// Surfaces that run before the work they guard: only a blocking fragment may attach here.
pub(super) const PRE_SURFACES: &[&str] = &["run.start", "node.before"];
/// Top-level fragment keys that would give it a timer, a store or a budget of its own.
pub(super) const FORBIDDEN_KEYS: &[&str] =
    &["timer", "store", "budget", "limits", "schedule", "cron"];
/// Node config keys that would give one fragment node a timer, a store or a budget.
pub(super) const FORBIDDEN_NODE_KEYS: &[&str] =
    &["budget", "store", "limits", "cron", "timer_ms", "in_ms"];
