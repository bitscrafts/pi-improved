# P4.7 "Boss" — opus adversarial review (pass 1)

**Date**: 2026-09-08 · **Reviewer**: claude-opus-5 (plan mode, 40-turn cap) · **Target**: `_docs/06-research/008-pii-self-improving-boss.md` @ `f8d515c`
**Verdict**: **APPROVE WITH CHANGES** · **Status**: review captured; action items to fold into P4.7 slice specs

> Full opus text rendered to stdout was truncated by the 40-turn cap; the
> load-bearing findings below were VERIFIED against the code by the orchestrator
> (not trusted verbatim) — each cites `path:line`.

## Single most important risk (resolve BEFORE building anything)

**The human approval gate is not actually a gate once a run resumes.** This
directly threatens the user's #1 decision (human must approve promotions):

- `spawn_resume` rebuilds capabilities WITHOUT `scope_root`
  (`crates/pii-server/src/runs.rs:134-136`) — a RESUMED scoped run continues
  with workspace-scoped caps, violating "scoped runs resume scoped."
- A gate node re-entered inside the same run reads its own cached decision and
  AUTO-ALLOWS: `approval_id = blake3(run_id, node_id)` (`nodes/gate.rs:88-103`),
  and on resume the id + cached `decision` (gate.rs:69-75) matches, so it skips
  the human.

**Every P4.7 slice that writes after an approval — which is all of Slice F
(dev/prod promotion) — is both unconfined and, on a second pass, unapproved.**
This is the top priority to fix.

## Action items (in order)

1. **Fix `spawn_resume` to persist + restore `scope_root` in the run header.**
   A scoped run must resume scoped. (`runs.rs:134-136` rebuilds caps fresh.)
2. **Make the promotion gate a fresh approval per cycle.** Either one run per
   cycle, or salt `approval_id` with a cycle/attempt counter — so a re-resumed
   gate always asks a human again, never auto-allows. Fix the
   `blake3(run:node)` cached-decision reuse (`gate.rs:66-103`).
3. **Fix `graphs/starter/heal-watch.toml:23`** — `gate = "quality-gate"` is NOT
   an `xtask` subcommand (`xtask/src/main.rs:172-175` → USAGE, exit 2), so the
   shipped watch graph records a **phantom permanent failure**.
4. **Add `reachability-check` + `doc-drift-check` to `run_ci`**
   (`xtask/src/main.rs:60-93`) and to `USAGE` — the two self-improvement gates
   aren't part of the CI gate set.
5. **Anti-reward-hacking refusal**: a boss patch whose `git diff --name-only`
   touches `xtask/`, `quality-gate.sh`, `graphs/`, `specs/`, `.github/`, or a
   `#[test]` assertion must ESCALATE instead of proposing (prevents the "make
   the gate pass by weakening it" incentive — the same trap the opus RSS review
   flagged).
6. **Drop Slice D as written** — a `route` node already exists and routes
   control flow, not models; model routing belongs in the agent/config layer,
   not a new node kind. **Drop the `pii-boss` crate from Slice C** — no new
   crate until a second consumer exists (the prior review's rule).
7. **Move dev to `/root/.pii-dev/` as a RUNTIME DIR ONLY; worktrees stay under
   the workspace** — because `validate_scope_root` (`runs.rs:205-227`) rejects
   any `scope_root` outside the workspace. A `/root/.pii-dev/` git worktree
   would be outside `/projects/pi-improved` and cannot be a heal scope_root.
   Keep worktrees under the workspace; `/root/.pii-dev/` is just the dev
   daemon's runtime/store dir.

## What this changes in the P4.7 build order

- **Slice A (silent-no-op gates) STAYS FIRST, plus a NEW prerequisite fix**:
  the `spawn_resume` scope_root persistence + the gate auto-allow fix are
  pre-existing bugs that Slice F (promotion) depends on. Fold them in early.
- **Slice B (boss loop skeleton) STAYS** — the cron-graph detect-and-escalate
  proof, but on an FIXED `heal-watch.toml` (item 3).
- **Slice C**: keep state.md / external-memory / compaction, but NO new
  `pii-boss` crate — implement in the existing graph/agent substrate.
- **Slice D**: model routing via agent config / the existing `route` node; NOT a
  new node kind.
- **Slice E/F**: patch/validate + promotion, AFTER the approval-gate fix (items
  1-2) so the human-approval contract actually holds.
- **Slice G/H**: research + GitHub unchanged; `pr` capability after a repo remote.

## VERDICT + user decision

**APPROVE WITH CHANGES.** The design's core (self-aware boss on its own substrate,
human-gated promotion, worktrees, model tiers) is right. The blocking risk is
the resume+approval-gate bug, which would have silently voided the user's
"human approves promotions" decision.

**USER OVERRIDE (2026-09-08): the boss IS a dedicated pii crate.** The user
directed twice — "a pii sub module!" then "crate!" — that P4.7 be a first-class
`pii-boss` crate (cohesive subsystem owning context-management + model-routing
+ the boss loop), in the same shape as `pii-web`/`pii-memory`/`pii-net`. This
OVERRIDES opus action item 6 ("drop the pii-boss crate — no second consumer").
The user's judgment stands: the boss is a large, cohesive, ongoing subsystem
that earns its own crate boundary. The `pii-boss` crate:
- owns the boss loop, state.md management, context compaction, model routing;
- builds on `pii-graph` (boss loop as graphs) + `pii-core`/`pii-server`
  (journal, gates, approvals, budget, memory) — same substrate rule;
- the opus prerequisite bugs (resume scope-root, approval-gate stale reuse,
  heal-watch phantom gate) still land in `pii-server`/`graphs`, but the BOSS
  logic lives in `pii-boss/`.

(Note: opus item 6 also said "drop the pii-boss crate" partly because a crate
boundary should be earned by a second consumer. The user has now asserted the
boss itself is that consumer class — the crate is the subsystem boundary, not
a speculative extraction.)
