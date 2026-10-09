//! `pii-evolve`: a thin CLIENT that launches the self-improving boss's watch
//! loop on the daemon (P4.7, spec 23/27).
//!
//! A **standalone, opt-in** crate: it is not a dependency of `pii-cli` or
//! `pii-server`, so the default `pii` binary neither links nor runs it — the
//! same delivery as `pii-web`/`pii-mcp` (a separate crate with its own
//! `pii-evolve` binary built only when an operator chooses to). It carries no
//! default feature; the boss code and its token/network risk surface are
//! opt-in by construction: an installer that does not build this crate ships
//! none of it.
//!
//! The boss LOOP is `graphs/starter/boss-watch.toml` — a real pii graph run
//! on the daemon, with a journal, a `gate` node, and the same approval
//! ledger every other suspending run uses. This crate does not run graphs
//! and never will: `driver::run_cycle` loads `state.md`, connects to the
//! daemon's UDS control socket, and sends one `run.start` for that graph,
//! then records the launched `run_id` back into `state.md`. No subprocess,
//! no `cargo xtask` shelling, no local gate evaluation.

#![forbid(unsafe_code)]

pub mod driver;
pub mod state;
