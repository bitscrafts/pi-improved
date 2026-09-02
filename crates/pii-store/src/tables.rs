//! redb table definitions (pass-1 §3.4 subset for Phase 1).
//!
//! Values are `serde_json` bytes. postcard is a later optimisation; it is not
//! needed to make the transaction property hold and it adds an encoding
//! failure mode a workhorse pass does not need.
//
// `pub(crate)` constants inside a private `mod tables` trip clippy
// `redundant_pub_crate`, but dropping the visibility makes them un-importable
// from `lib.rs`. Same exemption the xtask crate uses (spec guardrail F16).

#![allow(clippy::redundant_pub_crate)]

use redb::TableDefinition;

/// Scalar metadata (run counter, schema version).
pub(crate) const META: TableDefinition<'_, &str, u64> = TableDefinition::new("meta");

/// Serialised graph body, keyed by its blake3 hex hash.
pub(crate) const GRAPH_BODY: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("graph_body");

/// Run headers, keyed by run id.
pub(crate) const RUN: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("run");

/// Append-only event journal `(run, seq)` -> serialised event.
pub(crate) const RUN_JOURNAL: TableDefinition<'_, (&str, u64), &[u8]> =
    TableDefinition::new("run_journal");

/// Periodic blackboard snapshots `(run, seq)` -> serialised bytes.
pub(crate) const RUN_SNAPSHOT: TableDefinition<'_, (&str, u64), &[u8]> =
    TableDefinition::new("run_snapshot");

/// Status index `(status, run)` -> (), a secondary lookup for [`super::Store::runs_by_status`].
pub(crate) const RUN_BY_STATUS: TableDefinition<'_, (&str, &str), ()> =
    TableDefinition::new("run_by_status");

/// Resume key -> run id, the durable lookup for [`super::Store::put_resume_key`].
pub(crate) const RESUME_INDEX: TableDefinition<'_, &str, &str> =
    TableDefinition::new("resume_index");
