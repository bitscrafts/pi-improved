//! Engine-bridge concern: journal→bus bridge, run spawning, JSON-RPC
//! dispatch, session durability (spec 47 R1/R6).
//!
//! This module groups the code that bridges the daemon-independent
//! [`crate::Engine`] runtime to the outside world: the event [`bus`], run
//! [`runs::spawn_run`]/[`runs::spawn_resume`] lifecycle, the JSON-RPC
//! [`dispatch`] table, subscription bookkeeping ([`subscribe`]), node/lint
//! registries ([`registry`]), session durability ([`session_durability`]),
//! and the connection/transport plumbing (`conn`, `uds`, `loopback`,
//! `tool`).
//!
//! **Known coupling (DOCUMENTED, not yet resolved):** [`dispatch`] calls
//! into [`crate::engine::methods`] for ~10 JSON-RPC methods
//! (`WorkerList/Reload`, `ApproveList/Resolve`, `HealList/Get`,
//! `CronList`, `ChannelList`, `MemorySearch/Put/Consolidate`, `SysReload`,
//! `RunResume`). This is a real edge from the engine-bridge concern into
//! the daemon-app concern, and it is the reason spec 47 (F-13, Phase 5.2)
//! reorganizes module boundaries WITHOUT splitting `pii-server` into
//! separate crates: untangling this coupling requires either moving
//! routing logic or accepting a real crate dependency edge, and that
//! decision is explicitly deferred to a future opus-reviewed spec (see
//! amem key `pii/phase5.2/f13-scope-options-for-opus`). Anyone
//! considering the full crate split (`pii-engine`/`pii-nodes`/
//! `pii-daemon`) should start here.

pub mod bus;
pub mod conn;
#[cfg(test)]
mod conn_heartbeat_tests;
pub mod dispatch;
#[cfg(test)]
mod inline_measure_tests;
pub mod learn_methods;
pub mod loopback;
pub mod methods;
#[cfg(test)]
mod reconcile_tests;
pub mod refusal;
pub mod registry;
pub mod run_scope;
pub mod runs;
pub mod session_durability;
pub mod subscribe;
pub mod tool;
pub mod uds;
