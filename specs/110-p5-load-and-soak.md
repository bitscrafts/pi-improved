# Spec: 110 — P5: what it costs, under load and over time

**Project**: /projects/pi-improved
**Phase**: P5 of the road to production (`_docs/06-research/052-production-readiness-2026-10-09.md` §8).
**Goal**: the first MEASUREMENT of pii under load and over time, and of what specs 103-109 added to the cost
of every run. Nothing here is a feature; everything here is a number. **Measurement only — no tuning.**

## Why this spec exists (the honest state)
- Load has never been measured: no concurrency figure, no throughput, no latency percentiles, no leak check.
- Specs 103-109 added work to every run: a per-run trajectory JSONL (optional, default OFF), a rotated log
  file (optional), the observer fan-out to the daemon log (ON by default since 108), health polling, an
  audit log, and — in the GATE — an instrumented coverage build (~7.4G and minutes per run). None of that has
  a measured cost.
- The only performance numbers on record are the cold start (3-4 ms) and one one-shot run (5.5 s, dominated
  by a 3B local model). The user's own experience (where the seconds go) is unmeasured.

## Requirements
R1. **Load.** Drive the daemon with CONCURRENT runs for a bounded wall time (≥ 8 concurrent, ≥ 5 minutes).
    Use the **scripted model** (`PII_MODEL=scripted:…`) so the provider is not the variable — state that
    choice and its limit. Report: runs started/finished/failed, throughput (runs/s), and p50/p95/p99 for
    (a) the whole run lifecycle and (b) each RPC kind the metrics surface exposes. Take the numbers from
    `pii metrics` + `pii health --json` (counts), and wall-clock from the client, and say which source gave
    which number.
R2. **The instrumentation bill (103-109).** Same workload, same config, measured A/B:
    - trajectory sink OFF vs ON (`PII_TRAJECTORY_DIR` unset vs set);
    - log file OFF vs ON (`--log-file`);
    - observer attached vs `NullObserver` (if a switch exists; if not, say so — do not fake it).
    Report the delta in run latency, CPU and bytes written for each. If a delta is below the timer's
    resolution, say "below resolution" — do not invent a figure.
R3. **Soak.** ≥ 30 minutes continuous at a bounded concurrency. Report RSS at start / middle / end (from
    `pii health --json`), the store and journal growth, log and trajectory bytes on disk, and a LEAK VERDICT
    (does RSS trend up with runs completed?). If the budget cannot fit 30 minutes, run the longest you can,
    **state the actual duration**, and say explicitly what that does and does not prove.
R4. **The user's own path (the product number).** Break a one-shot `pii -p` run into startup, model time and
    tool time using the events already emitted; report the p50 over ≥ 5 runs and name the dominant term.
    This is the number a user feels.
R5. **The gate's new cost** (spec 108). Measure wall-clock and disk for `bash quality-gate.sh` with the
    coverage step, versus `QG_SKIP_COVERAGE=1`. The skip flag exists because the export needs ~25G; this is
    the price being paid for a real ratchet, and it should be a number.
R6. **Limits, stated plainly.** Single host, scripted model, no network variance, no multi-tenant load. List
    what was NOT measured (e.g. real provider latency, disk-full behaviour, many-channel fan-out).

## Exit Criteria
- [ ] R1 pasted: concurrency, duration, runs completed/failed, throughput, and the p50/p95/p99 table with its
      source named per number
- [ ] R2 pasted: the A/B deltas (or "below resolution" with the resolution stated) for all three components
- [ ] R3 pasted: the RSS series, store/journal/log/trajectory growth, the duration actually run, and a leak
      verdict that names the number it rests on
- [ ] R4 pasted: the one-shot breakdown (p50 over ≥ 5 runs) and the dominant term
- [ ] R5 pasted: gate wall-clock and disk for both modes
- [ ] R6 pasted: the list of what was NOT measured
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability` and `PASS: coverage-floor` (or the
      documented `QG_SKIP_COVERAGE=1` form, stated as such); `cargo run -q -p xtask -- fresh-gate --worktree`
      prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report is filed as `_docs/06-research/056-load-and-soak-2026-10-10.md` (measurements only)
- [ ] no daemon left running; your own scratch/exports deleted; say how much space you freed

## Guardrails
- **Measurement only.** If you find a performance defect, FILE it in the report with its number — do not fix
  it in this spec (a fix needs its own spec and its own controls).
- Do NOT change a threshold, a cap or a config default to make a number look better. If a cap is hit
  (trajectory truncation, log rotation, a queue limit), that is a FINDING: report it with the number.
- Do NOT leave unbounded growth running: the trajectory cap and the log rotation exist — use them, and say
  which bounded the run.
- The volume is at ~97% (19G free). Keep scratch small, reuse one scratch dir, delete your own exports, and
  report `df -h /projects` before/after.
- Take the numbers from the artefact the user would use (`pii metrics`, `pii health --json`, the log lines),
  not from a re-enactment.
- Do NOT commit; leave the tree for the orchestrator.
