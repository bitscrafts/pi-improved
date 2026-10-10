//! Daemon-owned node kinds (F5.9, F6.7): `gate` and `memory`.
//!
//! Registered alongside `agent`/`tool` in [`crate::engine::registry`] because both
//! need a resource the engine owns (the approval table, the memory store)
//! that `pii-graph` itself must not know about.

pub mod crash;
pub mod gate;
pub mod hal;
pub mod heal;
pub mod memory;
pub mod mqtt;
pub mod sop;
pub mod spawn;
pub mod worker;
