//! `pii-evolve`: state.md management, context compaction, and model
//! routing for the self-improving boss (P4.7, spec 23).
//!
//! A **standalone, opt-in** crate: it is not a dependency of `pii-cli` or
//! `pii-server`, so the default `pii` binary neither links nor runs it — the
//! same delivery as `pii-web`/`pii-mcp` (a separate crate with its own
//! `pii-evolve` binary built only when an operator chooses to). It carries no
//! default feature; the boss code and its token/network risk surface are
//! opt-in by construction: an installer that does not build this crate ships
//! none of it. The boss LOOP itself is a pii graph (a later slice); this crate
//! provides durable state (`state.md`) and the outer-loop skeleton that runs
//! the deterministic self-improvement gates and suspends for a human on red.

#![forbid(unsafe_code)]

pub mod driver;
pub mod state;
