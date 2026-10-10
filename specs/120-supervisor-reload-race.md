# Spec: 120 — a request must not be dropped by a reload (the supervisor race spec 119 surfaced)

**Project**: /projects/pi-improved
**Phase**: **Stage A5** of `057`. Spec 119 went looking for timing-sensitive TESTS and found a timing-sensitive
**PRODUCT**: `reload_drops_no_requests` fails under load because the supervisor can lose a request during a reload.
The test's assertion is right; the code is at fault. This is a data-loss defect in the daemon's own reload path.

## What was measured (2026-10-10, by spec 119 — reproduce it before changing anything)
- `crates/pii-worker/tests/supervisor.rs::reload_drops_no_requests` fails **6 of 25** runs at
  `taskset -c 0-3` with 16 CPU hogs, and failed 2 of 2 in a `cargo`-path run at that load.
- Observed errors: `WorkerDied`, and `worker writer closed` during a reload.
- Mechanism per a code read (NOT instrumented): `Supervisor::invoke` can read the supervisor state, then `reload`
  marks it draining and (seeing no registered requests) proceeds, and only then does the request register and get
  sent to a worker that is being shut down. The old worker's exit handling in `death()` may also contribute.
- **In production this is a lost request** — the operator sees a failure, or worse, a silent gap — during a reload.

## Requirements
R1. **Reproduce it DETERMINISTICALLY first.** A probabilistic repro (6 of 25 under load) cannot verify a fix. Add a
    test seam or use a barrier/channel to force the interleaving (e.g. pause `invoke` between reading the state and
    registering the request, or drive `reload` between those two steps). Paste the **deterministic** red.
R2. **State the mechanism in one line, after instrumenting** — log/observe the actual order of events in the failing
    interleaving. Spec 106's lesson applies: measure the evidence and verify the mechanism before writing it as fact
    (an earlier defect was described by a plausible mechanism that measurement disproved).
R3. **Fix the class, not the interleaving you happened to see.** A request that arrives while a worker is draining
    must be either (a) registered atomically with respect to the drain decision (same lock/state transition), or
    (b) **refused with a named error and a named event** — never silently dropped. State which you chose and why.
    Durability is non-negotiable: no request may vanish without a signal.
R4. **The existing test must pass with its assertion UNCHANGED**, at the same load: `taskset -c 0-3` with 16 hogs,
    **>= 25 runs, 0 failures** (paste). Plus the deterministic test red before, green after.
R5. **Observability**: a request refused/dropped because of a drain emits a name registered in `ALL` and is counted
    in the metrics; a completed reload reports **how many requests it drained** (so "reload happened and nothing was
    in flight" is distinguishable from "reload dropped something").
R6. **Also investigate `death()`**: does the old worker's exit handling contribute a second way to lose or double-handle
    a request? Answer either way with the evidence (a test or a stated non-reproduction).
R7. **Docs**: the runbook gains the reload semantics in one short section — what happens to a request that arrives
    during a reload, with the named event it produces.

## Exit Criteria
- [ ] a DETERMINISTIC reproduction exists (paste the seam and the red run), and the mechanism in one line
- [ ] the fix, with the existing test unchanged, passing **>= 25 runs at 4 cores + 16 hogs** (paste the count)
- [ ] the deterministic test red before / green after (paste both)
- [ ] a refused request emits its named event and its counter moves (paste); the name is in `ALL`
- [ ] a reload reports the number of requests it drained (paste)
- [ ] `death()` answered with evidence (paste)
- [ ] the runbook section exists (paste it)
- [ ] `bash quality-gate.sh` exits 0 (13 steps); plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists files, the new names, the load used, and what it could NOT reproduce

## Guardrails
- **Do not weaken, skip or ignore the existing test.** Its assertion is the specification here.
- Do not fix it by holding a global lock across the whole reload if that stalls requests for the reload's duration —
  state any latency cost you add, with the number.
- A silent drop is a fail, even if it makes the test green: the fix must leave a signal.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.
