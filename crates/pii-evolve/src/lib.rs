//! `pii-evolve`: state.md management, context compaction, and model
//! routing for the self-improving boss (P4.7, spec 23).
//!
//! A dedicated, feature-gated crate: it is not a dependency of `pii-cli`
//! or `pii-server`, so the default workspace build links none of it — an
//! installer that does not want self-improvement builds no boss code and
//! spends no boss tokens. The boss LOOP itself is a pii graph (a later
//! slice); this crate provides the three mechanisms a graph cannot express
//! on its own: durable state, context compaction, and auditable model
//! routing, plus the outer-loop skeleton that runs the deterministic
//! self-improvement gates and suspends for a human on red.

#![forbid(unsafe_code)]

pub mod context;
pub mod driver;
pub mod routing;
pub mod state;
