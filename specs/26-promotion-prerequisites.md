# pi-improved — spec-26: promotion-prerequisites

- **Project**: `/projects/pi-improved`
- **Priority**: The non-negotiable promotion contract. 008 §6/§7.1 and the user's
  #1 resolved decision ("human approves every promotion") are meaningless if a
  re-entered gate auto-approves itself or reads a stale decision. This spec makes
  "a human approval inside a loop is a fresh decision, every time" true, and gives
  the human a ledger that records what is actually being approved. Land BEFORE any
  boss-promotion loop (spec 27) or no one can approve it.
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: `_docs/06-research/009-pii-evolve-opus-review-pass1.md` items 1-2 +
  `_docs/06-research/011-pii-architecture-conformance-review-pass1.md` Part B
  spec 26 (fixes A17a, A17b, A17e).
- **Depends-On**: spec 24 (silent-noop-check) + spec 25 (pii-evolve honesty);
  green at HEAD.

---

## Verified Premises

Checked before writing (2026-09-09):

- `crates/pii-server/src/nodes/gate.rs:52-59` — `cycle` is read ONCE from static
  graph node config (`cfg.get("cycle").and_then(as_u64).unwrap_or(0)`). It is a
  static, build-time value.
- `crates/pii-server/src/nodes/gate.rs:83-86` — `approval_id(ctx, self.cycle)` =
  `blake3(run_id, node_id, cycle)[..16]` (gate.rs:96-100). With a static `cycle`,
  a gate re-entered on iteration 2 of the same run computes the SAME id, finds its
  OWN cached decision (gate.rs:76-81), and returns `allow` — no fresh human.
- `crates/pii-server/src/daemon/approvals.rs:23-35` — `resolve_approval(id, allow,
  note, now_ms)` is the decision path; `resumed` reflects a newly-applied decision.
- `crates/pii-store/src/heal.rs:13-38` — `HealAttempt`/`HealAttemptRun` are the
  Slice-0 shape: `outcome: String` ("detected"/"flake"), `attempts: Vec`, no
  `patch_hash`, no `gate_before`/`gate_after`, no `decision`, no cost fields,
  unbounded `attempts`.
- `crates/pii-server/src/runs.rs:43` — `validate_scope_root(workspace, root)` is
  called from `spawn_run`. `spawn_resume` (runs.rs:111) restores `scope_root` from
  the journal (spec 20 R1, via `stored_run_scope_root` runs.rs:138) but NEVER
  re-validates it (validate_scope_root appears once: runs.rs:43).
- `crates/pii-store/src/approval.rs:13-36` — `Approval { id, policy, prompt,
  decision: Option<bool>, ... }`; `put_approval`/`get_approval`/`resolve_approval`.
- `crates/pii-store/src/codec.rs` — `to_bytes`/`from_bytes` (CAS/bincode) used by
  every store; `HealAttempt` is `Serialize + Deserialize` (heal.rs:8-19).
- redb `Database` used per-store; heal.rs:52-57 sets a 4 MiB cache (small store).

---

## Overview

008 (and the user) are explicit: promotions are human-approved, never auto. Two
code realities violate that:

1. **A gate re-entered inside a loop auto-approves itself.** `cycle` is static
   graph config; iteration 2 of the same run re-reads the cached decision. The
   boss loop's exact topology (a `gate` node inside a `for`/iteration) therefore
   never asks a human twice. This is A17a.
2. **The heal ledger has nothing to approve FROM.** It records `outcome: String`
   and an unbounded `attempts: Vec` — no `patch_hash` (what changed), no
   `gate_before`/`gate_after` (did it help), no `decision` (who approved, against
   which approval id), no cost (`model`/tokens/micro_usd). A human asked to
   "approve this promotion" has no record of the patch it will apply or the
   before/after evidence.
3. **`spawn_resume` never re-validates `scope_root`** — a resumed scoped run may
   point at a path that is no longer inside the workspace, silently widening (or,
   on later code, mis-directing) confinement. A17b.

This spec makes the gate's `cycle` a **runtime** decision (a loop-iteration or a
per-attempt value folded into the id at run time — not a static config read),
evolves `HealAttempt` into a ledger a human can actually approve from, and
re-validates `scope_root` on resume.

---

## Requirements

### Functional

- **R1 (runtime approval cycle)**: `GateNode::run` derives the cycle salt at RUN
  TIME from the node input, not only from static config. Concretely: a gate node
  reads `cycle` from (a) an `input.payload.cycle` value if present, ELSE (b) the
  static `cfg["cycle"]`, defaulting to 0. The static-config path stays byte-identical
  (cycle 0 = historical id, §8.8 replay intact). A loop graph that increments
  `cycle` per iteration (in a transform before the gate) thus gets a FRESH
  `approval_id` and a FRESH human decision on every iteration.
- **R2 (fresh decision is observable)**: when a re-entered gate computes a cycle
  that differs from the run's already-decided cycle, it must NOT return the cached
  `allow` — it must suspend again with the new `approval_id`. The existing cache
  lookup (gate.rs:76-81) is keyed by id, so a different id already skips it; the
  spec's job is to make the id differ and to TEST it.
- **R3 (heal-ledger evolution)**: evolve `HealAttempt` per 010 §C / 011 A17e:
  - `outcome: String` → a typed `HealState` enum (`Detected`, `Flake`, `Fixed`,
    `Applied`, `Escalated`), replaced as a serialized enum;
  - add `patch_hash: Option<String>` (content-addressed via the existing CAS,
    never stored inline), `worktree: Option<String>`, `branch: Option<String>`,
    `base_commit: Option<String>`;
  - add `gate_before: Option<String>` / `gate_after: Option<String>` (the raw gate
    exit/evidence, so "did it help" is recorded);
  - add `decision: Option<ApprovalDecision>` where `ApprovalDecision { allow: bool,
    approval_id: String, by: String, ts_ms: u64 }` — bound to the id `gate.rs` mints;
  - add cost `model: Option<String>`, `tokens_in`, `tokens_out`, `micro_usd` (u64);
  - `attempts: Vec<HealAttemptRun>` becomes a bounded ring (default capacity 20)
    with a separate `attempt_count: u32` so total attempts are never lost to the
    ring eviction.
- **R4 (scope_root re-validation on resume)**: `spawn_resume` calls
  `validate_scope_root(workspace, root)` on the recovered `scope_root` before
  resuming; an out-of-workspace or relative root fails the resume loudly
  (`PiiError::Invalid`), never silently resumes widened.
- **R5 (backward-compatible store)**: existing `heal.redb` rows (old `outcome:
  String` shape) must `deserialize` under the new types (serde `#[serde(default)]`
  + enum with a catch-all or a migration read path), so upgrading the daemon does
  not abort on a historical row.

### Non-Functional

- **N1**: the quality gate stays green; the test count only goes up.
- **N2**: no new third-party dependencies (redb/serde already in pii-store).
- **N3**: cycle 0 behavior is byte-identical to today (§8.8 replay: suspend →
  approve → resume still re-reads its own decision).
- **N4**: `silent-noop-check` (spec 24) stays green; new fields/types must have a
  reader (a reader here = the ledger view / approval path actually reads them).

---

## Architecture

```
gate node (re-entered in a loop)
   transform increments input.payload.cycle   <- R1: runtime, per-iteration
        -> GateNode::run(cycle = payload.cycle | cfg.cycle | 0)   <- R1
             approval_id = blake3(run,node,cycle)                  <- fresh per cycle
                id not in store  -> Suspend (fresh human on EVERY cycle)  <- R2
                id in store      -> cached allow/deny

HealAttempt (evolved, R3)
   HealState enum | patch_hash(CAS) | worktree/branch/base_commit
   | gate_before/gate_after | ApprovalDecision{allow,approval_id,by,ts_ms}
   | model/tokens_in/tokens_out/micro_usd | attempts: ring(20) + attempt_count

spawn_resume -> validate_scope_root(recovered root) BEFORE resuming  <- R4
```

**Key decision — cycle is a RUNTIME value, not a rename of the static config.**
The divergence in gate.rs is that `cycle` is bound at graph-build time. The fix is
to let the node read `input.payload.cycle` at run time (falling back to config,
then 0). This keeps the static config (some graphs want a fixed cycle) while making
the boss loop's per-attempt freshness a data-flow decision. It is one small change
to `GateNode::run` + `approval_id` already takes the cycle; the TDD test proves
iteration-2 differs from iteration-1.

**Key decision — the ledger is *the thing a human approves from*.** The Approval
object carries policy+prompt; the HealAttempt carries the patch/before/after/decision
evidence. These are two records joined by `approval_id`: `Approval.id ==
HealAttempt.decision.approval_id`. A promotion approval thus has, in one join, the
human decision AND the patch + before/after evidence + cost. boots 008 §6's
"provenance: every boss-authored commit carries a revertible marker."

**What this spec is not**: it does NOT build the boss loop (spec 27), does NOT wire
model routing (spec 28). It makes the promotion contract hold and the ledger
auditable.

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `gate_cycle_from_input_overrides_config` | a GateNode with `cfg["cycle"]=1`; `input.payload.cycle=5` | `approval_id` uses cycle 5 (different from cycle-1 id) |
| R1 | `gate_cycle_defaults_to_config` | `input.payload` has no cycle; `cfg["cycle"]=3` | cycle 3 used |
| R1 | `gate_cycle_zero_is_historical` | no input cycle, no config cycle | cycle 0 = the same id as today's `blake3(run,node)` |
| R2 | `reentered_gate_fresh_cycle_suspends_again` | run a loop graph: iter1 cycle 1 approved; iter2 input sets cycle 2 | iter2 SUSPENDS (new id), does NOT return cached allow |
| R3 | `heal_attempt_round_trips_evolved` | a HealAttempt with HealState::Fixed, patch_hash, gate_before/after, decision, cost | serialize → deserialize → all fields equal |
| R3 | `attempt_ring_caps_and_counts` | push 25 attempts into a capacity-20 ring | `attempts` len == 20, `attempt_count` == 25 |
| R4 | `spawn_resume_rejects_out_of_workspace_root` | a run journal with `scope_root=/etc` | resume returns `PiiError::Invalid`, run NOT resumed |
| R4 | `spawn_resume_accepts_valid_root` | a run journal with `scope_root` inside the workspace | resume proceeds |
| R5 | `legacy_heal_row_deserializes` | an old `outcome:"detected"` row bytes | deserializes to `HealState::Detected` (non-fatal) |

**R2 is the acceptance test.** A plausible implementation "adds cycle to the id"
but reads it only at build time still passes a static test and fails the loop
correction. The test MUST drive a two-iteration loop graph through one gate node
and assert iteration 2 suspends with a DIFFERENT approval id (not `allow`). If a
static `cfg.cycle` were the only mechanism, iter2 would reuse iter1's approved id —
the test fails. R3's ring and R4's resume-reject are the other two must-fail-naively
tests (a naive `Vec` keeps 25 attempts; unvalidated resume accepts `/etc`).

---

## Exit Criteria

Machine-checked (each >=1 test, non-vacuity `grep -qE "ok\\. [1-9]"`):

- [ ] `cargo test -p pii-server reentered_gate_fresh_cycle_suspends_again` — the R2 acceptance test passes (R1,R2)
- [ ] `cargo test -p pii-server gate_cycle_` — cycle source/override/zero tests pass (R1)
- [ ] `cargo test -p pii-store heal_attempt_ ` — evolved round-trip + ring tests pass (R3)
- [ ] `cargo test -p pii-server spawn_resume_` — scope_root re-validation tests pass (R4)
- [ ] `cargo test -p pii-store legacy_heal_row_deserializes` — historical row compatibility passes (R5)
- [ ] `cargo test --workspace` — 0 failures; count >= baseline (N1)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N1)
- [ ] `cargo fmt --all --check` — clean (N1)
- [ ] `cargo xtask deps-check` + `net-check` + `silent-noop-check` — pass (N4)

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The two `approval_id`s from the R2 test pasted — they must differ.
3. A legacy-heal-row fixture byte sample + its parsed `HealState` shown (R5).

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT change cycle-0 behavior** (N3): absent input cycle AND absent
  config cycle MUST produce the historical `blake3(run,node)` id, else §8.8 replay
  breaks.
- **G5 — do NOT remove the Approval ↔ HealAttempt join** — `decision.approval_id`
  must equal the gate-minted id (R3).
- **G6 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G7 — report raw output, never summed totals.**

### Error handling expectations

- `gate.rs` must fail LOUDLY if a cycle value is present but unparseable (a non-u64
  `input.payload.cycle`) — never silently fall back to 0 and reuse a stale decision.
- `spawn_resume` with an invalid recovered `scope_root` must ERROR the resume, not
  resume unconfined (R4).
- A legacy heal row that cannot map to any `HealState` must surface as a skipped
  row + a written warning in the reader, never abort the whole list.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-server/src/nodes/gate.rs` | `run()` reads `input.payload.cycle` then `cfg["cycle"]` then 0 (R1); tests R2 acceptance |
| `crates/pii-store/src/heal.rs` | `HealState` enum, `patch_hash`/`worktree`/`branch`/`base_commit`, `gate_before/after`, `ApprovalDecision`, cost fields, bounded ring + `attempt_count` (R3) |
| `crates/pii-store/src/heal.rs` | serde defaults + legacy-enum catch-all so old rows read (R5) |
| `crates/pii-server/src/runs.rs` | `spawn_resume` re-validates recovered `scope_root` (R4) |
| `crates/pii-server/src/daemon/approvals.rs` | (if needed) expose `approval_id`/decision so the ledger records the bound id (R3 join) |

**Not modified**: `pii-proto` wire method surface, `pii-cli`, `pii-mcp`, `pii-web`,
`pii-evolve` (spec 25 handles that crate). HealStore stays redb + 4 MiB cache
(rss-gate); only the row schema evolves, reading old rows.