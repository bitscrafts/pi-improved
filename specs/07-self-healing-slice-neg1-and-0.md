# Spec: Pii Self-Healing Module — Slice -1 + Slice 0

**Project**: /projects/pi-improved
**Phase**: 3.6 — Self-healing foundation (Slice -1: reachability gate;
Slice 0: detect-and-escalate)
**Spec of record**: `_tmp/phase3/opus-heal-and-rss-response.md` §Part B
(opus architecture review, 2026-09-05) — the build order, guardrails,
and rationale below are taken directly from that review, not invented.
Also consult `_tmp/phase3/self-healing-concept.md` (the original draft
opus reviewed) for context, and `_tmp/phase3-opus-review.md` (the
Phase-3 deep review) for the M12/B1/M3 findings Slice -1 targets.

## Overview
This is the FIRST two slices of a larger self-healing/self-improving
module ("use itself to fix itself" — user directive). Per the opus
review, do NOT build patching, worktrees, git tools, or PR tools yet —
those are Slice 1+ and explicitly out of scope here. This spec covers
ONLY:
- **Slice -1**: a deterministic (no AI, no model calls) `xtask` gate
  that catches two classes of bug mechanically: (a) a capability/
  module declared but never reachable from a non-test call path (the
  exact shape of B1 — `ollama::provider_from_env` had zero non-test
  callers — and M3 — `Supervisor::invoke` was test-only), and (b) a
  factual claim in HANDOFF.md contradicted by a machine-checkable fact
  (a lighter version of the "honest gap list" the opus Phase-3 review
  produced by hand).
- **Slice 0**: a cron-triggered graph that runs the existing quality
  gates, and on failure: records the failure signature to a new
  ledger table, re-runs the SAME check once to confirm it's
  reproducible (not a flake), and suspends on a `gate` node for human
  acknowledgement. **No patching. No worktree. No `fs_write`
  capability granted to this graph at all.** This proves the full
  substrate (cron → graph → gate execution → journal → ledger →
  approval-gated suspend/resume) with zero model risk and zero write
  authority, before any future slice spends a token on repair.

## Requirements

### Functional

#### S(-1).1 — `xtask reachability-check`
- New `xtask` subcommand, alongside `deps-check`/`net-check`/
  `lint-files`. Purpose: for a configurable list of "declared capability
  -> expected non-test caller" pairs, assert the caller actually
  exists in non-test source.
- Implementation approach: a static list of `(symbol_path, must_be_
  called_from_glob)` pairs checked via text search (`grep`-equivalent)
  across non-test `.rs` files for the symbol's call site — keep this
  cheap and dependency-free (note `deps-check`'s own check, read
  `xtask/src/deps.rs`, actually parses `cargo metadata` JSON for its
  dependency-graph invariants; this new check is simpler — pure text
  search is sufficient since it's checking "does this string appear
  as a call outside tests", not graph structure).
- Seed the initial list with the TWO already-known historical misses
  so the gate is provably not vacuous: (a) `pii_model::ollama::
  provider_from_env` must have a caller outside `#[cfg(test)]`/
  `tests/` (this is NOW true since B1 landed — `pii-server/src/lib.rs`
  calls `resolve_with_transport` which calls it — so this entry
  should PASS today, proving the gate would have caught B1 before it
  was fixed, not that it's currently broken); (b)
  `pii_worker::Supervisor::invoke` — check whether this NOW has a
  non-test caller (per the M3 finding it may still only be test-only;
  if so this entry legitimately FAILS today and that failure is
  expected/documented, not a bug in the gate).
- Exit 0 if every entry's caller exists; exit 1 with
  `REACHABILITY FAIL: <symbol> has no non-test caller` per violation
  (same message style as `deps-check`'s `DEPS-CHECK FAIL: ...`).
- Add to the seed list going forward as new capabilities are added
  (this is a living list, not a one-time check) — document this in
  the module's doc comment.

#### S(-1).2 — `xtask doc-drift-check`
- New `xtask` subcommand. Purpose: catch ONE concrete class of
  doc/code mismatch — a HANDOFF.md claim of the form "X works" /
  "X exists" that a cheap machine check can contradict.
- Scope narrowly for v1: check that every crate path or file path
  mentioned in HANDOFF.md inside backticks (e.g. `` `crates/pii-x/
  src/y.rs` ``) actually exists on disk. This is deliberately the
  simplest possible doc-drift check — do not attempt semantic claim
  verification (e.g. parsing "Discord works" and checking it against
  test results) in this slice; that is real NLP/semantic work out of
  scope. A file-existence check alone would have caught nothing new
  this session, but it's the correct FIRST rung of doc-drift checking
  and the anchor future checks build on.
- Exit 0 if every backtick-quoted path in HANDOFF.md that LOOKS like a
  repo-relative path (starts with `crates/`, `specs/`, `xtask/`,
  `graphs/`, `scripts/`, `fixtures/`, `_tmp/`, or is a bare filename
  ending in a known extension) resolves to an existing file/dir; exit
  1 listing each dangling reference.

#### S0.1 — `pii-store` gains a `heal_attempt` table
- New table, same pattern as `approval`/`channel_cursor`
  (`pii-store/src/approval.rs`/`channel.rs` — follow that file's
  structure: a small `HealStore` struct wrapping `Database`, open/
  put/get methods, no held transaction).
- Schema: `heal_attempt: signature_hash(&str) -> encoded HealAttempt`
  where `HealAttempt { signature_hash: String, gate: String,
  first_seen_ms: u64, last_seen_ms: u64, attempts: Vec<HealAttemptRun>
  }` and `HealAttemptRun { run_id: String, outcome: String,
  ts_ms: u64, detail: String }`. `signature_hash` =
  `blake3(gate_name, first_N_bytes_of_failure_output)` — deterministic
  so re-detecting the SAME failure updates the same row instead of
  creating a new one (this is what the future rate-limiter in Slice 1+
  will query).
- Methods: `put_attempt`/`get_attempt`/`list_attempts` (mirrors
  `ApprovalStore`'s naming).
- New file `pii-store/src/heal.rs`, registered in `pii-store/src/
  lib.rs` alongside `approval`/`mem`/`timer`/`channel`.

#### S0.2 — `heal` node kind (or reuse `tool`? — see Guardrails)
- A new node kind, `heal_detect`, registered ONLY in the server
  registry (like `gate`/`memory`), NOT in `pii-graph`'s builtin
  registry — same pattern B2 already established for `agent`/`tool`/
  `gate`/`memory`.
- Config: `{ gate: String }` (which `xtask` subcommand to run, e.g.
  `"quality-gate"`, `"lint-files"`, `"reachability-check"`,
  `"doc-drift-check"`).
- Behavior: shells out to `cargo xtask <gate>` (via the EXISTING
  `ProcCap`/`bash` mechanism — do NOT invent a new process-exec path;
  reuse what `runs.rs:root_caps` already grants for `/bin/sh`). On
  exit 0: port `ok`. On non-zero exit: re-run the SAME command once
  (reproduction check); if it fails BOTH times, write a `HealAttempt`
  row to the `heal_attempt` table (via a new `Ctx`-scoped store
  handle, following how the `memory`/`gate` nodes get their store
  reference through their factory) and route to port `detected`; if
  the second run passes (flake, not reproducible), route to port
  `flake` and do NOT write to the ledger.
- Ports: `ok`, `detected`, `flake`.
- Required caps: `PROC` only (to shell out to `cargo xtask`) — **no
  `FS_WRITE` beyond what running `cargo xtask` itself needs, no `NET`**
  (see Guardrails — this node must not be able to write source files).

#### S0.3 — `graphs/starter/heal-watch.toml`
- Entry: `timer` node, `cron = "*/30 * * * *"` (every 30 min — matches
  the existing `consolidate.toml` cron cadence pattern) → port `fired`
  → `check` (`heal_detect`, `gate = "quality-gate"`) → port `detected`
  → `escalate` (`gate` node, `policy = "heal-escalation"`, `prompt`
  describing the failure) → suspends for human ack; port `ok`/`flake`
  → `done` (`transform`, no-op).
- Must pass `pii graph lint`.
- This graph is started once at daemon boot per F5.8's existing cron
  wiring pattern (one run per firing, independently traceable) — no
  new daemon wiring needed beyond adding this file to `[[cron]]` in a
  config, which is already-existing F5.7/F5.8 machinery.

#### S0.4 — `pii heal ls` / `pii heal show <signature_hash>`
- New `pii-cli` subcommands, mirroring `pii approve ls` exactly
  (`cmd_ctl.rs`/wherever `approve ls` lives — follow that file's
  structure). `heal ls` lists open (unacknowledged) heal_attempt
  entries; `heal show <hash>` prints one entry's full attempt history.
- New protocol methods `Method::HealList`/`Method::HealGet`, following
  `Method::ApproveList`'s exact pattern (`pii-proto/src/method.rs`:
  enum variant + `"heal.list"`/`"heal.get"` string mapping in both
  directions, same as `ApproveList`/`"approve.list"`).

### Non-Functional
- No file exceeds 450 effective lines (`cargo xtask lint-files`).
- No new third-party crates — `heal_detect`'s process-exec reuses
  existing `ProcCap`; the ledger reuses existing `redb`/`pii-store`
  patterns; nothing here needs a new dependency.
- `unsafe_code = "forbid"`; existing lint policy unchanged.

## Guardrails (from the opus review — non-negotiable)
- **The `heal_detect` node MUST NOT be granted `FS_WRITE`.** This
  slice does zero patching by design; if a future slice adds a
  patching node kind, it must be a SEPARATE node with its OWN capset,
  never blur `heal_detect` (read-only detection) and a future
  `heal_patch` (writes, scoped to a disposable worktree only) into one
  node or one capability grant.
- **No auto-anything.** Every detected failure suspends on a `gate`
  node for human acknowledgement. There is no code path in this slice
  that resumes a suspended heal-escalation automatically.
- **Reproduce before recording.** `heal_detect` must re-run the SAME
  check and get the SAME failure before writing to the ledger — a
  single flaky run must never create a ledger entry (this is the
  "require reproduction" guardrail from the AIOps research, applied
  literally).
- **Do not let this slice edit `specs/`.** Same rule as every other
  workhorse in this project.
- **This slice must not attempt to fix the RSS-growth bug or use it as
  an example/fixture.** Per the opus review's explicit finding: that
  bug has no localization signal and its cheapest "fix" is weakening a
  forbidden threshold — it is exactly the wrong first case for this
  system. If a demo/fixture failure is needed for testing, use a
  deliberately broken unit test in a scratch location instead.

## Exit Criteria
- [ ] `cargo build --workspace --all-targets`
- [ ] `./quality-gate.sh` (fmt + clippy -D warnings + test, all PASS)
- [ ] `cargo xtask lint-files`
- [ ] `cargo xtask deps-check`
- [ ] `cargo xtask net-check`
- [ ] `cargo xtask reachability-check` (new gate itself must pass or
  fail with a clear, documented message — not crash)
- [ ] `cargo xtask doc-drift-check` (same)
- [ ] `pii graph lint graphs/starter/heal-watch.toml`
- [ ] `cargo test -p pii-store` includes new tests for `HealStore`
  put/get/list round-trip
- [ ] `cargo test -p pii-server` includes a new test for `heal_detect`
  proving: (a) a passing gate routes `ok` and writes nothing to the
  ledger; (b) a failing gate that fails BOTH times routes `detected`
  and writes exactly one ledger row; (c) a gate that fails once then
  passes (simulated flake) routes `flake` and writes nothing
- [ ] `pii heal ls` (manual or scripted smoke check) runs without error
  against an empty ledger and against a ledger with one entry

## Guardrails (repeat, mandatory)
- Do NOT weaken any existing gate, test, or lint threshold to satisfy
  an exit criterion above.
- Do NOT edit this spec file or any file under `specs/` — you are the
  workhorse, not the planner.
- Checkpoint-commit clean-building work between exit criteria if you
  hit a tool-call/iteration budget mid-task.
