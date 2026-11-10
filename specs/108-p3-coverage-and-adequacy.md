# Spec: 108 — P3: measure coverage, and decide what each subsystem MUST make observable

**Project**: <repo>
**Phase**: P3 of the road to production (`_docs/06-research/052-production-readiness-2026-10-09.md` §8) —
"close the quality gaps in what already ships". Two deliverables, and the second is the honest one.

## Context
- **Coverage has NEVER been measured** (052 §9 and every assessment since): there is no tooling, so
  "613 tests pass" says nothing about which code they exercise.
- **Observability adequacy is a FLOOR, not a proof.** Specs 100/101 gave every non-exempt crate one
  named-event test; that proves a seam exists and fires in that test, NOT that the crate emits what an
  operator needs to diagnose a failure. No lint can decide this — it is a per-subsystem judgement, and this
  spec is where it gets made.
- **Two residuals from spec 107 to fold in here** (they are small and belong with the P3 quality pass):
  1. `pii -p` (one-shot) still defaults to the `off` sandbox profile while the daemon defaults to
     `workspace` — the interactive path is the unconfined one.
  2. Bash deny-list hits are NOT emitted as `tools.*` events; only `core.cap.denied` appears.

## Requirements
R1. **Measure coverage, per crate, and report the numbers.** Use a real tool (`cargo-llvm-cov` is the
    natural one; `cargo tarpaulin` is an alternative). If the tool cannot be installed or cannot run here,
    that is a FINDING: paste the exact error and say what is missing — do not substitute a proxy metric and
    call it coverage.
    - Report a table: crate · lines covered / lines total · percent, and the workspace total, measured on
      the same run.
    - Say which test scopes are counted (unit + integration + doc, and whether `--lib`-only or full).
R2. **A ratchet, not a target.** Add a coverage FLOOR that fails the gate when the total drops below it
    (start a few points under the measured value so it does not flap), with the measured number pinned in
    the spec and the gate step printed as a `PASS:`/`FAIL:` line like its siblings. If the tool cannot run
    in the gate for a real reason (time, availability), say so and record what would be needed — an
    unenforced number is a report, not a ratchet.
R3. **The adequacy review, per subsystem — and then CLOSE the gaps you find.** For each crate that emits
    (15 today), answer: *"if this subsystem fails, what must an operator see to diagnose it, and does it
    emit that today?"* Produce a table `crate → failure mode → the event/measure that reveals it → emitted?
    → evidence (test or run)`. Where the answer is "nothing reveals it", that is a gap: emit it (through
    `pii-observe`, names registered in `ALL`) and pin it with a test that can fail. Do not write a gap you
    cannot then test; do not test a name nothing emits.
    - Non-exempt crates with NO defensible failure-mode answer must be named in the report — an honest
      "nothing here needs an event, and this is why" is a valid answer, silence is not.
R4. **Fold in the two spec-107 residuals** above: the one-shot default, and the bash deny-list event.
    Each gets a test; the one-shot default is a real behaviour change, so state what it breaks if anything.
R5. **A mutation control for anything you add** (this repo's standing rule): for each new assertion, show a
    mutation that makes it RED, or state why no mutation is possible.

## Exit Criteria
- [ ] coverage measured and pasted: the per-crate table + the workspace total, with the exact command and
      tool version; if the tool could not run, the exact error and what is missing
- [ ] the floor exists as a gate step (paste `PASS:`/`FAIL:` output) and a test/mutation shows it FAILS when
      the threshold is raised above the measured value (paste)
- [ ] the adequacy table exists for every emitting crate, with a one-line answer each (emitted / not needed
      and why / gap emitted now)
- [ ] every gap you close has a test that can fail (paste one mutation)
- [ ] R4: the one-shot sandbox default is decided and tested, and a bash deny-list hit emits its event
      (paste both)
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate
      --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists every file changed and every new `names.rs` const

## Guardrails
- **A number nobody enforces is decoration.** Either the floor fails the gate, or it is reported as an
  unenforced measurement with the reason.
- Do NOT lower a threshold to make a run green; if the measured value changed because code moved, say so.
- Coverage is not correctness: state plainly that a high number with weak assertions is still weak.
- Emit through `pii-observe`; no new raw prints beyond the file's budget.
- Scratch under `<scratch>/`; delete your own exports (a coverage run can be large; one fresh-gate
  export is ~8G). Do not leave a daemon running.
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09/10)
**Coverage was measured for the first time, and it is ENFORCED.** Report:
`_docs/06-research/054-coverage-and-adequacy-2026-10-10.md` (the worker put the table there because
`specs/` is read-only for it — correct call).

**Reproduced by me:**
```
target/coverage-summary.json → lines 21737 / 28234 = 76.99%   (cargo-llvm-cov 0.9.1)
coverage-floor control:  --floor 74.0 → rc=0   --floor 78 → rc=1   --floor 90 → rc=1   ← it can fail
quality-gate.sh step:    "coverage-floor: total 77.01% >= floor 74.00%  ·  PASS: coverage-floor"
the gate MEASURES each run (it invokes cargo llvm-cov, own target dir) — the floor is not read from a file
MUTATION (mine): removing the run-chain tool tap (runs.rs:242) → log_wiring's
  `a_deny_list_hit_in_a_run_reaches_the_log` FAILED at log_wiring.rs:215; restored → green
R4 tests: oneshot_default_sandbox 3 passed · log_wiring 7 passed · suite total 765 passed, 0 failed
```
**The adequacy review found the real gap — and it was the one that mattered:** most observability seams
FIRED IN UNIT TESTS BUT WERE NEVER ATTACHED IN THE DAEMON, so in production nothing was written. Now wired
(stores, model provider, network, workers, channels, every tool call + bash deny-list hit, memory search +
consolidate, TUI failures), each with a mutation control.

**Declared open, with reasons (NOT closed):**
1. **`pii_proto::client::Client::connect` HANGS FOREVER if the peer closes the socket without answering the
   hello** — found by accident, not fixed. A hang has no error, no exit code and no log. **This is the
   subject of spec 109** (reproduce, bound the handshake, report).
2. Graph metrics still don't reach the log (flooding risk; the journal records them) → spec 109 R3.
3. The memory search inside an agent turn stays unobserved and swallows its error → spec 109 R4.
4. `HEALTH_STATE_CHANGED` and `OBSERVE_EVENTS_DROPPED` are registered and emitted NOWHERE — dead names in
   the vocabulary → spec 109 R2.
5. `pii-http-fetch` has no dependents; `pii tui` still defaults to the `off` sandbox → spec 109 R5.

**Trade-off to state plainly:** the gate now runs an instrumented build (~7.2G of `target/llvm-cov-target`
and minutes per run), and `fresh-gate` pays it again in the export. Accepted: an unenforced number is
decoration, and the disk guard still refuses below `FRESH_GATE_MIN_FREE_MB`.

## PAUSE NOTE (2026-10-10, user paused the session)
Committed with ONE criterion open, stated honestly: **`fresh-gate --worktree` was started twice and stopped
both times by the orchestrator to protect the disk** (it now pays the instrumented coverage build inside the
export — measured 18G in ~5 minutes, which took `/projects` to 99% with 9.5G free). The REPO gate is green
(`quality-gate.sh` ALL PASS, including `PASS: coverage-floor` at 77.01% >= 74.00%) and every other criterion
was reproduced by the orchestrator. **Resume with:**
```
cd <repo> && FRESH_GATE_MIN_FREE_MB=6000 cargo run -q -p xtask -- fresh-gate --worktree
```
(afterwards: `df -h /projects`; if it failed, the export is kept under `<scratch>/fresh-gate-<pid>` and
is 18G — remove it to reclaim space before re-running).

### The open criterion is now CLOSED (2026-10-10, orchestrator run)
```
cargo run -q -p xtask -- fresh-gate --worktree   (FRESH_GATE_MIN_FREE_MB=6000)
  … PASS: reachability-check · quality-gate: ALL PASS · fresh-gate: PASS   FG-RC=0
  df after: 594G 541G 28G free (96%)
```
So spec 108 is complete: the repo gate AND the CI-model fresh-tree gate are both green, including the new
`PASS: coverage-floor` step measured inside the export. The disk guard was raised to 6G for the run because
the instrumented coverage build inside the export is ~18G; it passed with 28G free and the export was
reclaimed by the tool.

