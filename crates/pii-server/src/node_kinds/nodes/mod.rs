//! Daemon-owned node kinds (F5.9, F6.7): `gate` and `memory`.
//!
//! Registered alongside `agent`/`tool` in [`crate::engine::registry`] because both
//! need a resource the engine owns (the approval table, the memory store)
//! that `pii-graph` itself must not know about.

pub mod agent_claude;
pub mod agent_quota;
pub mod agent_route;
pub mod agent_worker;
pub mod classify;
pub mod coder;
pub mod coder_container;
pub mod coder_output_keep;
pub mod coder_runtime;
pub(crate) mod coder_runtime_api;
pub(crate) mod coder_runtime_cli;
pub(crate) mod coder_runtime_util;
pub mod coder_skills;
pub mod crash;
pub mod criteria;
pub mod entity;
pub mod gate;
pub mod hal;
pub mod heal;
pub mod judge_spec;
pub mod memory;
pub mod mqtt;
pub mod post;
pub mod queue;
pub mod sop;
pub mod spawn;
pub mod wasm;
pub mod worker;
