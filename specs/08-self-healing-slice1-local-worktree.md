# Spec: Pii Self-Healing — Slice 1 (local-only worktree repair)

**Project**: <repo>
**Phase**: 3.6 — Self-healing (Slice 1 of the opus-recommended build order)
**Spec of record**: `_tmp/phase3/opus-heal-and-rss-response.md` §Part B —
this spec implements exactly its "Slice 1" paragraph: *"localize + patch
+ validate in a disposable worktree, against a deliberately broken unit
test. Write capability scoped by `PathScope` to the worktree only."*
The user selected the **local-only** variant: no remote push, no PR
(Slice 2/3 deferred until the B4 TLS/rustls gap is fixed, which also
unblocks `heal-pr`). Building on the ALREADY-SHIPPED Slice -1 + 0
(`specs/07-self-healing-slice-neg1-and-0.md`, committed).

## Overview

Slice 0 proved detect → ledger → escalate with zero write authority.
Slice 1 adds the first WRITE capability, confined so it cannot hurt the
main repo: a heal attempt runs against a **disposable git worktree**
inside the workspace; the run's `fs_write` PathScope is **the worktree
only**; the agent that patches has **no bash tool** (fs/edit tools only,
which all go through `Ctx::fs()` and are therefore mechanically bound by
the scope — the opus "make it a capability, not a policy" guardrail); a
human gate node approves or denies the result; nothing is ever pushed or
merged automatically. Slice 1 ends with a reviewed patch in
`_tmp/heal-reviews/` and a trash-can'd worktree.

This slice also encodes opus's hard warnings:
- The seeded failure is a **deliberately broken unit test** — never the
  RSS bug (no localization signal; cheapest fix is the forbidden
  threshold weakening; reproduction costs minutes). That class stays a
  Slice-0 *escalate-to-human* finding forever, by design.
- **The healer may never replace the running daemon binary.** Deploy
  stays human, permanently. Nothing in this slice produces an artifact
  that could be executed in place of the daemon; the deliverable is a
  *patch file for human review*, full stop.

## Requirements

### Functional

#### S1.1 — `xtask heal-seed` (deterministic broken test in a disposable worktree)
- New `xtask` subcommand, mirroring the gate-runner style of
  `reachability-check`/`doc-drift-check` (module `xtask/src/heal_seed.rs`,
  wired in `main.rs`, registered in `USAGE`).
- Behavior: `cargo xtask heal-seed` →
  1. Compute signature `sig = blake3(<UTC timestamp>, <git HEAD>)` short
     form (first 12 hex chars) — human-readable, not a UUID.
  2. Create a git worktree of the CURRENT repo at
     `<workspace>/_tmp/heal-worktrees/<sig>` on a new branch
     `heal/<sig>` (via `git worktree add -b heal/<sig> <path> HEAD` —
     invoke git through `std::process::Command`, same as `rss_gate.rs`
     already does; xtask is dev tooling, the Command ban does not apply
     there).
  3. Apply ONE deliberate, deterministic break to a single source file in
     the WORKTREE: flip a numeric literal in a `const`/`static` that an
     existing unit test asserts (implementer picks the target by
     grepping `assert_eq!`-against-const in `crates/pii-core` tests;
     the flip must make exactly ONE test fail, naming the const, with a
     `file:line` in its output). Record the original line so the seed is
     reversible and verifiable.
  4. Record the seed in a small JSON ledger
     `<workspace>/_tmp/heal-seeds.json`: `{sig, worktree, branch, ts_ms,
     broken_file, broken_line, original_line}`.
  5. Print one line: `heal-seed: worktree=<path> sig=<sig> test=<test
     name> break=<file>:<line>`; exit 0 iff the seeded break was verified
     to fail: run `cargo test -p <crate> <test> --manifest-path
     <worktree>/Cargo.toml` ONCE and confirm exit != 0 (this is the
     reproduction check; the seed refuses to exit 0 if the break did not
     actually break).
- Rate limit + disk ceiling (opus "stop condition"/"worktree lifecycle"
  findings, enforced at the seed boundary — the system only attempts a
  fix when a human/xtask initiates one):
  - Refuse (exit 1, message) if the same `sig` already exists in
    `heal-seeds.json` within the last 24 h (2 attempts per signature per
    24 h ceiling, then escalate-by-refusal).
  - Refuse if `<workspace>/_tmp/heal-worktrees/` holds >= 50 entries or
    totals > 2 GiB (`du -sb`); the operator must run `heal-prune` first.
- Cleanup NEVER `rm -rf`s (hard project rule): `heal-prune` (S1.5) moves
  worktrees to `<workspace>/_tmp/heal-trash/` before any removal, and
  removes the git worktree registration (`git worktree remove` is fine —
  it is git's own non-destructive-relative mechanism; on failure fall
  back to `mv` the directory to trash).

#### S1.2 — Run-scope override: `run.start` gains optional `scope_root`
- The capability-confinement change. `Engine::start_run` and
  `spawn_run`/`spawn_resume` (crates/pii-server/src/runs.rs, dispatch.rs)
  currently build the run's `CapSet` from `root_caps(&engine.cfg.
  workspace)` → `fs_write = PathScope([workspace])`. This slice threads
  an OPTIONAL per-run scope root through:
  - `RunStartParams` (proto/cli) gains `"scope_root": string?` (absolute
    path). Wire method unchanged (`run.start`), params object extended.
  - Validation: when present, `scope_root` MUST be an absolute path
    inside `engine.cfg.workspace` (lexical + canonical check); otherwise
    `run.start` returns an error (`InvalidParams`-class message naming
    the violation). Default `None` → today's behaviour, byte for byte.
  - Extract the cap construction into a pure function
    `run_caps(workspace: &Path, scope_root: Option<&Path>) -> CapSet`
    used by both spawn paths:
    - `None`: `fs_read = PathScope(["/"])`, `fs_write =
      PathScope([workspace])`, `proc_argv0 = ["/bin/sh"]` (current).
    - `Some(root)`: `fs_read = PathScope([root, workspace])` (read the
      worktree + read-only visibility of the repo), `fs_write =
      PathScope([root])` ONLY, `proc_argv0 = ["/bin/sh"]`.
  - Every non-heal caller of `start_run`/`spawn_run` passes `None` (cron
    firing, oneshot `pii -p`, any other call site found by grep — list
    them in the commit).
- Unit tests for `run_caps`: assert `Some(root)` yields `fs_write`
  denying a path in `workspace/specs/…` and allowing a path under `root`;
  assert `None` yields today's scopes.

#### S1.3 — `graphs/starter/heal-fix.toml` (localize → patch → validate → escalate)
- Graph shape (lint-clean; reuse only EXISTING node kinds — agent, tool,
  route/transform, gate — no new kinds per opus's "no new crate/node
  kind for v1"):
  1. `probe` — `tool`, `tool = "bash"`, cwd from input; command from
     input (`$.input.test_command`, e.g. `cargo test -p pii-core
     <seeded_test> --manifest-path <worktree>/Cargo.toml`) — this is the
     LOCALIZE step's input: the failing test name + worktree come from
     `heal-seed`'s output, passed as graph input.
  2. `gate_fail` — `route` on `$.payload.exit_code`: `eq 0 → port ok`
     (nothing broken — escalate "unexpected green" to the gate node for
     human eyes, do NOT silently pass), else → `fail`.
  3. `fix` — `agent`, `loop_budget = 2`, `tools = ["fs_read",
     "fs_write", "edit", "multi_edit"]` — **NO bash** (mechanical
     confinement; the agent can only touch paths its run's `fs_write`
     PathScope allows = the worktree). System prompt tells it: the
     failing test output is in `$.payload`; localize via the compiler's
     `file:line`; make the minimal source change in the worktree to make
     the test pass; do NOT touch any file outside the worktree.
  4. `retest` — `tool` bash, same test command as `probe`.
  5. `gate_pass` — `route` on exit_code: `eq 0 → ok`, else → `fail`.
  6. `feedback` — `transform` building `{"feedback": <test output>,
     "worktree": ..., "attempt": ...}` → edge back to `fix` (the
     verify-fix retry pattern, proven in Phase 2).
  7. `summarize` — on pass: `tool` bash running
     `git -C <worktree> diff --stat && git -C <worktree> diff` →
     payload carries the full patch text.
  8. `escalate` — `gate` node, policy `"heal-approval"`, prompt
     embedding sig/worktree/test + the patch; suspends for human ack
     (approve/deny). This is the ONLY terminal success path.
  9. `fail` terminal (`transform`) when `fix` exhausts `loop_budget`.
- The run is started with `scope_root = <worktree>` and input
  `{worktree, test_command, sig}` — one RPC: `pii run start
  --graph starter/heal-fix --input '{"scope_root": "<worktree>",
  "worktree": "<worktree>", "test_command": "cargo test -p pii-core
  <test> --manifest-path <worktree>/Cargo.toml", "sig": "<sig>"}'`
  (exact CLI spelling per existing `run start` arg parsing — check and
  match it; if `run start` has no input flag, extend the CLI
  minimally following its existing flag style).
- Must pass `pii graph lint` (V5 terminal-port warnings acceptable).

#### S1.4 — `xtask heal-collect <sig>` (the ONLY way a fix leaves the sandbox)
- After a heal run reaches `escalate` and a human APPROVES (via the
  existing approval mechanism), the operator runs:
  `cargo xtask heal-collect <sig>` →
  1. Refuse if no approval record exists for the run (check the
     approval table through the same path `pii approve ls` uses, or if
     that is server-only, require an explicit `--approved <run_id>`
     flag that the operator copies from the approval output — pick the
     cheaper correct option and document it).
  2. `git -C <worktree> diff > <workspace>/_tmp/heal-reviews/<sig>.patch`
     (creates the dir; the patch is the full deliverable).
  3. Print the patch path + the seed's `original_line` (so a human can
     verify the fix restored the flipped constant) + `heal-seed` ledger
     row.
- Exit 0 iff the patch file was written and is non-empty.

#### S1.5 — `xtask heal-prune` (worktree lifecycle + disk budget)
- Moves every entry under `_tmp/heal-worktrees/` older than 7 days (or
  all, with `--all`) to `_tmp/heal-trash/` via `mv` (NEVER `rm -rf`);
  removes each from git's worktree registry first (`git worktree
  remove <path> --force` when the tree is dirty is allowed — git's own
  removal is not a destructive filesystem command; but if that errors,
  plain `mv` still happens). Prints a one-line summary.
- `heal-trash/` itself is emptied by the operator manually; this tool
  never deletes.

### Non-Functional
- No file over 450 effective lines (`cargo xtask lint-files`).
- No new third-party crates; no new node kinds; no new wire methods
  (params extended only). New Rust = `run_caps` refactor + scope
  threading + three small xtask modules.
- `unsafe_code = "forbid"`; workspace lints (`-D warnings` clippy,
  pedantic/nursery as configured) must pass — run the REAL gate
  (`cargo clippy --workspace --all-targets -- -D warnings`), not plain
  `cargo build`, before declaring done (the Slice -1/0 worker lost a
  full cleanup cycle to this exact mistake).
- The heal agent NEVER has `bash`; graph `tool` bash nodes run only
  human-authored commands from the graph file, never model-generated
  commands.
- The running daemon binary is never replaced by any artifact of this
  slice. The deliverable is a reviewed `.patch` file. If the fix target
  ever touches `pii-server`/`pii-cli` daemon code, `heal-collect`
  prints a note: "patch touches daemon code — build and deploy remains
  human, per project rule".

## Guardrails (opus Part B, quoted intent)
- Make confinement a **capability, not a policy**: the run's `fs_write`
  PathScope IS the worktree; `specs/` edits in the main repo are
  mechanically impossible for the heal agent, not merely discouraged.
- **Do not seed the RSS bug.** The seeded break is a unit test with
  `file:line` localization and a sub-minute reproduction. If a spec or
  a worker proposes using rss-gate as the Slice-1 target, stop and
  escalate to the planner — that choice is forbidden by the review's
  four independent reasons (violates reproduce-before-patch economics,
  no localization signal, likely "not a code defect" judgment call, and
  the cheapest fix is weakening the forbidden `SLOPE_KB` threshold).
- Budget: each `fix` agent attempt is bounded by the graph's declared
  `limits.max_tokens` (B6 wires it) AND `loop_budget`; no attempt may
  run unbounded. (Known gap recorded by opus: `max_model_calls` does
  not exist yet — do NOT build it in this slice; note it in the commit
  message as still-open.)
- **Reproduce before recording** (carried from Slice 0): `heal-seed`
  verifies the break fails before exiting 0; the graph re-runs the test
  after each fix attempt.
- This slice must not edit `specs/` or `HANDOFF.md` (workhorse rule).
- Cleanup is `mv` to trash, never `rm -rf` (hard project rule; also a
  disk-budget mandate, not optional — opus finding 7).

## Exit Criteria
- [ ] `cargo build --workspace --all-targets`
- [ ] `./quality-gate.sh` (fmt + clippy `-D warnings` + test, all PASS)
- [ ] `cargo xtask lint-files` / `deps-check` / `net-check` /
  `reachability-check` / `doc-drift-check`
- [ ] `cargo xtask heal-seed` — creates worktree, applies break, prints
  the one-line summary, and the seeded test is VERIFIED failing (exit 0
  only after that verification); re-run within 24 h → exit 1 with the
  rate-limit message
- [ ] `cargo test -p pii-server` — new tests: `run_caps` scope
  construction for `Some(root)` (denies `<workspace>/specs/…`, allows
  `<root>/…`) and `None` (current behaviour); `run.start` rejects
  `scope_root` outside the workspace
- [ ] `pii graph lint graphs/starter/heal-fix.toml`
- [ ] `cargo xtask heal-collect <sig>` (after a scripted end-to-end:
  seed → run heal-fix with a scripted model that restores the flipped
  constant via `multi_edit` → approve → collect) writes a non-empty
  `.patch` whose content equals the reverse of the seed's recorded
  `original_line` flip
- [ ] `cargo xtask heal-prune` moves the worktree to trash and prints a
  summary; the main tree is byte-identical to HEAD before and after the
  whole Slice-1 exercise (`git status --porcelain` empty apart from
  `_tmp/` artifacts)
- [ ] A scripted end-to-end transcript of seed → fix → approve →
  collect → prune exists under `_tmp/phase3/slice1-e2e.log` showing
  every step above

## Notes for the implementer (read first)
- Specs of record to consult: `specs/07-self-healing-slice-neg1-and-0.md`
  (Slice-0 patterns to mirror: xtask module style, gate message style,
  ledger schema), `_tmp/phase3/opus-heal-and-rss-response.md` §Part B
  (the authoritative design), `crates/pii-tools/src/fs.rs` (all fs
  tools route through `Ctx::fs()` — the enforcement point), 
  `crates/pii-server/src/runs.rs:100-165` (`root_caps`, `spawn_run`,
  `spawn_resume` — the refactor site), `crates/pii-server/src/
  dispatch.rs:149` (`run_start`), Phase-2's `scripts/phase2/
  selfchange.sh` (the proven repo-copy fix pattern this slice makes
  first-class).
- The DeepSeek worker does NOT see `-D warnings` clippy errors with
  plain `cargo build` — run the real gate every iteration cycle.
- Keep the spec file and `specs/` read-only; commit checkpoints between
  exit criteria; prefer a clean, small, honest slice over gold-plating.
