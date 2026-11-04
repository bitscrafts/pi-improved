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

/// Memory documents: doc id -> encoded `Doc` (F4.2).
pub(crate) const MEM_DOC: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("mem_doc");

/// Postings `(term, doc_id)` -> term frequency (F4.5: redb-resident, no
/// in-RAM inverted index).
pub(crate) const MEM_TERM: TableDefinition<'_, (&str, &str), u32> =
    TableDefinition::new("mem_term");

/// Document frequencies: term -> number of docs carrying it.
pub(crate) const MEM_DF: TableDefinition<'_, &str, u64> = TableDefinition::new("mem_df");

/// Quantised i8 vectors + f32 scale: doc id -> packed bytes.
pub(crate) const MEM_VEC: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("mem_vec");

/// Concept co-occurrence `(concept, doc_id)` -> weight.
pub(crate) const MEM_EDGE: TableDefinition<'_, (&str, &str), u64> =
    TableDefinition::new("mem_edge");

/// Scalar corpus metadata: `n_docs`, `sum_dl`, `dims`.
pub(crate) const MEM_META: TableDefinition<'_, &str, u64> = TableDefinition::new("mem_meta");

/// Archived (never deleted) documents: doc id -> encoded `Doc`.
pub(crate) const MEM_ARCHIVE: TableDefinition<'_, &str, &[u8]> =
    TableDefinition::new("mem_archive");

/// Per-doc layer byte (additive, F4.12 `same_layer_only`): doc id -> layer
/// tag. Absent for pre-existing docs; readers fall back to L1.
pub(crate) const MEM_LAYER: TableDefinition<'_, &str, u8> = TableDefinition::new("mem_layer");

/// Skill evidence (additive, spec 210): doc id -> `ms` as u64 LE, then the reference bytes. The
/// `(commit-or-file, ms)` a `pii skill edit` recorded; absent for a doc never edited through it.
pub(crate) const MEM_EVIDENCE: TableDefinition<'_, &str, &[u8]> =
    TableDefinition::new("mem_evidence");

/// Timer wheel `(fire_at_ms, run_id)` -> resume key (F5.5).
pub(crate) const TIMER_WHEEL: TableDefinition<'_, (u64, &str), &str> =
    TableDefinition::new("timer_wheel");

/// Approvals `approval_id` -> encoded record (F5.10).
pub(crate) const APPROVAL: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("approval");

/// Heal-attempt ledger (S0.1).
pub(crate) const HEAL_ATTEMPT: TableDefinition<'_, &str, &[u8]> =
    TableDefinition::new("heal_attempt");

/// Channel cursor + last started run: channel -> encoded `{cursor,run_id}`.
pub(crate) const CHANNEL_CURSOR: TableDefinition<'_, &str, &[u8]> =
    TableDefinition::new("channel_cursor");

/// Spec 370: the agent-call totals record: `"totals"` -> encoded `SysStatus`, maintained by every run write.
pub(crate) const SYS_STATUS: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("sys_status");

/// Spec 370: one contribution row per run: run id -> encoded per-run row the record is summed from.
pub(crate) const SYS_STATUS_RUN: TableDefinition<'_, &str, &[u8]> =
    TableDefinition::new("sys_status_run");

/// Session META blobs: session id -> encoded `{cwd,tools,graph,status,
/// run_id}` (part 1 substrate; part 2/D3-D6 narrowed this to meta-only —
/// the transcript lives in [`SESSION_MSG`] so a status/`run_id` change never
/// re-serialises the growing message list).
pub(crate) const SESSION: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("session");

/// Cumulative `(input, output)` token counts per session — a separate table
/// so token updates never deserialise the whole session blob.
pub(crate) const SESSION_TOKENS: TableDefinition<'_, &str, (u64, u64)> =
    TableDefinition::new("session_tokens");

/// Append-only session transcript `(session_id, seq)` -> encoded
/// `WireMessage` (D3/D6, 026 report 5.0.4): one row per message, written
/// exactly once. Replaces the pre-fix behaviour of re-serialising the
/// WHOLE (monotonically growing) transcript on every turn, which made
/// cumulative bytes written O(n^2) over a session's lifetime.
pub(crate) const SESSION_MSG: TableDefinition<'_, (&str, u64), &[u8]> =
    TableDefinition::new("session_msg");

/// Periodic full-session snapshots `(session_id, seq)` -> encoded full
/// `Session` bytes (D3/D6, 026 report 5.0.4), taken every
/// `SESSION_SNAPSHOT_EVERY` appended messages so a restart/reseed replays
/// only the [`SESSION_MSG`] tail after the latest snapshot instead of the
/// session's entire history. Mirrors [`RUN_SNAPSHOT`]'s pattern.
pub(crate) const SESSION_SNAPSHOT: TableDefinition<'_, (&str, u64), &[u8]> =
    TableDefinition::new("session_snapshot");
