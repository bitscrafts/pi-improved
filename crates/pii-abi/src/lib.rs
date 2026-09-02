//! Kernel ABI: traits and types shared by every later Pii crate.
//!
//! Phase 0 ships only the scaffolding surface - error type, id newtypes,
//! effect classification, and the synchronous node metadata trait. The
//! async `Node::run` contract arrives in Phase 1.

#![forbid(unsafe_code)]

mod effects;
mod error;
mod ids;
mod node;

pub use effects::Effects;
pub use error::PiiError;
pub use ids::{NodeId, PortName, RunId};
pub use node::{NodeInput, NodeMeta, Outcome};
