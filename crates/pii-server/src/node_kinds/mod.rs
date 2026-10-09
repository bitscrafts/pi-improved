//! Server node-kinds concern: the graph node kinds `pii-server` registers
//! beyond `pii-graph`'s built-ins (spec 47 R2/R6).
//!
//! Groups `agent` (spec 43's split: turn loop, tool wiring, memorize/
//! compact support) and `nodes` (`crash`/`gate`/`heal`/`memory`/`spawn`/
//! `worker`), plus `compaction` (agent-support, used only by
//! `agent::turn`). See [`crate::engine`] for the module-boundary map and
//! the documented `dispatch`→`engine::methods` coupling this reorg does
//! NOT resolve.

pub mod agent;
pub mod compaction;
pub mod nodes;
