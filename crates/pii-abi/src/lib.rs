//! Kernel ABI: traits and types shared by every later Pii crate.
//!
//! Pure types only — this crate carries no runtime dependencies beyond the
//! serde/schemars ceiling and must never gain a new one.

#![forbid(unsafe_code)]

mod effects;
mod error;
mod ids;
mod journal;
mod node;

pub use effects::Effects;
pub use error::PiiError;
pub use ids::{NodeId, PortName, RunId};
pub use journal::{RunEvent, RunHeader, RunStatus, Terminal};
pub use node::{NodeInput, NodeMeta, Outcome, ResumeKey, SuspendReason};
