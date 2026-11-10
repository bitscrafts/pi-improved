# Spec: 73 — boss_watch waits on condition, not on a load-sensitive clock

**Project**: <repo>
**Phase**: batch B (from 043 §10) — candidate 7: "a gate that goes red under load teaches people to re-run until green"
**Status**: NOT STARTED
**Derives from**: `_docs/06-research/043-opus-full-review-2026-10-06.md` §1 (step-4 failure:
`boss_watch_reaches_detect_after_resume ... timed out waiting for journal growth` at load
22.8; reran 10/10 clean as load fell) + tonight's re-observation (quality-gate.sh run red,
passes alone at load 7)

## Overview

`boss_watch.rs`'s poll helpers (`wait_for_status` :183, `wait_for_journal_len` :200) use a
fixed `Duration::from_secs(10)` deadline while polling every 10 ms. Under load (the gate
runs the whole workspace suite on 32 cores; scratch builds push load >20), a resume that
normally completes in ~7 s legitimately takes >10 s — the deadline, not the system, fails.
The test then fails INTERMITTENTLY in the shared gate: the exact re-run-until-green hazard.

## Requirements

### R1 — deadline scales with environment, not constants
Both helpers take their deadline from an env override with a generous default:
`PII_TEST_DEADLINE_SECS` (default 60, ≥3× the worst observed loaded time). The env var is
read once per helper call.

### R2 — poll interval is honest
Keep the 10 ms poll (a slow poll would mask real deadlocks); no other behaviour change.

### R3 — failure message carries the evidence
On timeout, the panic message includes: the condition, the deadline used, and the journal
length last seen (for `wait_for_journal_len`) — a future failure must self-describe.

### R4 — guardrails
No production-code changes (this is the test file only); no new deps; the 8 boss_watch
tests still pass individually AND under `cargo test -p pii-server --test boss_watch` with
`PII_TEST_DEADLINE_SECS=5` on an idle machine (control: the override is honoured, proving
R1's read path).

## TDD Contract (ACCEPTANCE)

| Test | Input | Expected |
|---|---|---|
| `deadline_override_is_honoured` (NEW, red today) | run `boss_watch_reaches_detect_after_resume` with `PII_TEST_DEADLINE_SECS=5` on idle machine | passes AND the run does not regress (proves the env path) — implement by asserting the helper reads the var (e.g. a helper unit check or an instant < 5 s bound) |
| existing 8 tests unchanged-behaviour | gate rerun at load | no intermittent timeout at load ≤ 22 (the practical claim: the same workload that failed tonight now has 6× headroom) |

## Exit Criteria

- [x] `PII_TEST_DEADLINE_SECS=5 cargo test -p pii-server --test boss_watch 2>&1 | grep -q 'test result: ok. 8 passed'` (verified 3× consecutive + under the full suite; probe test ships as `#[ignore]` = the 9th counted test is absent by design — see HANDOFF envelope)
- [x] default-deadline run: `ok. 8 passed; 0 failed; 1 ignored` — and the FULL quality-gate.sh passes (the flake that killed three gate runs today is closed)
- [x] `grep -c 'PII_TEST_DEADLINE_SECS' crates/pii-server/tests/boss_watch.rs | grep -qv '^0$'`
- [x] `cargo fmt --all --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings` (workspace-wide per the direct-pi worker's run; -p pii-server alone passes too)

## Implementation Notes (worker)

- The flake mechanism is measured and named in 043 §1 — do NOT re-derive; read that section.
- Do not touch journal/engine code.