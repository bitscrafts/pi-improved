//! Graph engine: spec model, TOML loader, expression language, node harness
//! ABI, and static validator (spec F4).
//!
//! Phase 1 so far covers the deterministic "lint" surface — model, loading,
//! expression evaluation, and validation. The scheduler, blackboard, and run
//! entry points land in a dedicated pass.

#![forbid(unsafe_code)]

pub mod blackboard;
pub mod expr;
pub mod journal;
pub mod node;
pub mod nodes;
pub mod run;
pub mod sched;
pub mod spec;
pub mod toml_load;
pub mod validate;

pub use node::{GraphNode, NodeFactory, NodeRegistry};
pub use spec::{Edge, EdgeKind, GraphLimits, GraphSpec, JoinPolicy, NodeSpec, RetryPolicy};
pub use toml_load::load_toml;
pub use validate::validate;
