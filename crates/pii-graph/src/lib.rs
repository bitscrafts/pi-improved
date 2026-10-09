//! Graph engine: spec model, TOML loader, expression language, node harness
//! ABI, and static validator (spec F4).
//!
//! Phase 1 so far covers the deterministic "lint" surface — model, loading,
//! expression evaluation, and validation. The scheduler, blackboard, and run
//! entry points land in a dedicated pass.

#![forbid(unsafe_code)]
// Spec 77: a discarded `must_use` result is how the journal's flush loss hid.
// Enabled here (the crate the defect lived in); the gate's `-D warnings` makes
// any new discard a hard failure. Intentional sites carry a named `allow`.
#![warn(clippy::let_underscore_must_use)]

pub mod blackboard;
pub mod cron;
pub mod expr;
pub mod journal;
pub mod node;
pub mod nodes;
pub mod render;
pub mod run;
pub mod run_helpers;
pub mod sched;
pub mod spec;
pub mod toml_load;
pub mod validate;

pub use node::{GraphNode, NodeFactory, NodeRegistry};
pub use spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
pub use toml_load::{load_toml, load_toml_with_registry};
pub use validate::validate;
// observability: none — pending the spec 100 adoption sweep (no seam yet)
