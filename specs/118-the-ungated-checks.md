# Spec: 118 — the three ungated repo checks become gate steps (after the one that lies is fixed)

**Project**: /projects/pi-improved
**Phase**: **Stage A3** of `057` — a check that runs nowhere enforces nothing. Owner's standing preference:
**mandatory enforcement over an advisory observer.**

## What was measured (2026-10-10, on the untouched tree)
`quality-gate.sh` has nine reported steps, and three `xtask` checks exist that are **not** among them:

| check | result on HEAD | what it is |
|---|---|---|
| `deps-check` | **FAILED** `pii-proto: direct dependency 'pii-observe' is not in the allowlist` | the repo's own dependency invariants (`xtask/src/deps.rs`: 5 numbered invariants — crate purity, no redb outside `pii-store`, no rustls outside `pii-net`, single versions, client crates must not pull `pii-graph`) |
| `net-check` | **FAILED** `xtask/src/lint_observability.rs:27 — UnixStream::connect` | a scan for I/O/socket primitives that should not appear where they are |
| `doc-drift-check` | **FAILED** "`_tmp/sidecar.log` (HANDOFF.md) does not exist" | docs naming paths that do not exist |

Two of the three are real: `deps-check` caught a genuine invariant break (spec 109's contract dep on `pii-proto`,
since resolved by making it optional behind the `client` feature). **`net-check` is a FALSE POSITIVE** — line 27 of
`lint_observability.rs` is a **doc comment** that *mentions* `UnixStream::connect` while documenting spec 109 R8.
The check scans source text without excluding comments, so it reports a violation that does not exist.

## Requirements
R1. **Fix `net-check`'s false positive first.** It must not flag text inside line comments, block comments or doc
    comments. **Control both ways**: (a) the current `lint_observability.rs:27` hit disappears; (b) a REAL
    `UnixStream::connect` introduced into a crate's *code* (not a comment) still fails the check, and so does one
    introduced into a comment-adjacent position that is still code — paste both.
R2. **Clear `doc-drift-check`'s current finding honestly** — either fix the reference in `HANDOFF.md` or teach the
    check that a scratch path under `_tmp/` is not a durable doc target (state which, and why).
R3. **Promote all three to gate steps** in `quality-gate.sh`, each printing `PASS: <name>` / `FAIL: <name>` like
    the existing steps, and **fail-closed**: a check that cannot run is a FAIL, never a skip.
R4. **Controls for each promoted step** (paste): introduce a violation → the gate goes red naming it; restore →
    green. For `deps-check` the violation can be a temporary non-optional dep; for `net-check` a real socket call
    in a crate; for `doc-drift-check` a doc naming a nonexistent path.
R5. **Docs**: `_docs/02-guides/quality-gate-and-fresh-tree.md` gains the three steps, what each forbids, the
    invariants `deps-check` encodes (list all five with one line each), and the updated step count.
R6. **Keep the gate usable**: state each step's wall-clock contribution; if any is slow enough to hurt the fresh
    tree, say so with the number rather than hiding it.

## Exit Criteria
- [ ] `cargo xtask net-check` exits 0 on HEAD; a real socket call in a crate's code makes it exit non-zero (paste
      both, plus the comment-case control that must stay green)
- [ ] `cargo xtask deps-check` exits 0 (invariant 2 preserved by the optional-dep fix); a temporary non-optional
      violation makes it exit non-zero (paste)
- [ ] `cargo xtask doc-drift-check` exits 0 (paste)
- [ ] `bash quality-gate.sh` exits 0 and now prints `PASS: net-check`, `PASS: deps-check`,
      `PASS: doc-drift-check` (paste the summary lines and the new step count)
- [ ] each of the three controls above pasted, each with the restore
- [ ] the guide lists the three steps + the five `deps-check` invariants (paste the section)
- [ ] `fresh-gate` (plain) prints `fresh-gate: PASS`; clippy `-D warnings` and `fmt --check` exit 0
- [ ] the report states each step's wall-clock cost and what it does NOT cover

## Guardrails
- **Do not weaken a check to make it pass.** If a rule is obsolete, that is a report line and a spec proposal, never
  a silent deletion of the rule or of the step.
- A promoted step must be deterministic and offline: no network, no dependence on a scratch path existing.
- Do not increase the gate's runtime without stating the cost; the fresh tree already builds the workspace twice.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.

## Result + orchestrator verification (2026-10-10) — 7/8, and my spec's two measurements were WRONG
**What landed (verified by me):** the three checks are gate steps and the gate now has **13** steps; my runs:
`net-check` rc=0, `deps-check` rc=0, `doc-drift-check` rc=0. The worker's own violation→red controls are pasted in
its report (a real `UnixStream::connect` in code, a non-optional `pii-graph` dep in `pii-tui`, a doc naming a
nonexistent file — each turns the gate red, all restored) and the cost is **+2-3 s per gate run** (each check ~1 s
warm against a ~4.5-minute gate).

**MY SPEC UNDERSTATED BOTH PROBLEMS, and the worker measured the truth:**
- `net-check` did not have *one* false positive: it had **nine hits** — the doc-comment FP I found, plus **eight
  real socket uses**. Seven were allowlisted with stated reasons (one entry had gone stale when `service.rs` was
  split); and **one is real production code: `crates/pii-evolve/src/driver.rs` dials the daemon socket directly**
  instead of going through `pii_uds::client_connect`. It was allowlisted with a comment rather than fixed, and the
  guide says so. That fix changes error text and adds a crate edge → **its own spec** (filed below).
- `doc-drift-check` had **37** findings, not one — all in `HANDOFF.md`: 36 were legitimately ignored once the
  checker learned that gitignored `_tmp/` scratch, globs, `…`/`...` and `<N>` placeholders are not durable
  targets (and that `:5-9` is a line range), and **three had genuinely drifted** and were corrected.
- Comment stripping now blanks line/doc/nested-block comments while **keeping string and char literals** (so a
  `"//"` cannot hide code) — with a test written first (`net_check_ignores_comments`).

**Criterion 8 (fresh-gate) is NOT met, and the reason is a defect class, not this spec:** the worker's two runs
failed **different** tests — first `resume_never_reinvokes_completed_node`, then `diamond_race_cancels_loser` and
`diamond_race_cancels_multi_node_arm`. Tests that fail differently run to run under gate load are timing-sensitive
checks, and this lane treats a check that reports failures nobody can trust as a defect. Filed as **spec 119**
(Stage A4) with the repro harness I have running: `cargo test --workspace` (what the gate runs) under CPU pressure.

