//! Daemon-owned node kinds (F5.9, F6.7): `gate` and `memory`.
//!
//! Registered alongside `agent`/`tool` in [`crate::registry`] because both
//! need a resource the engine owns (the approval table, the memory store)
//! that `pii-graph` itself must not know about.

pub mod crash;
pub mod gate;
pub mod memory;
