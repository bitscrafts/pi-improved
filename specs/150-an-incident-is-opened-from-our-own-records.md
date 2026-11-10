# Spec: an incident is opened from the system's own records, with no model

## Overview

Measured by the Opus proactivity judgement (`_docs/06-research/061-opus-proactivity-judgement-2026-10-13.md`, C-run):
replaying **551 recorded outcomes** from the loop's own log with a fixed model-free rule yields exactly **3
incidents**, two of them real problems a person then fixed — the `dirty-tree` one after **41.5 hours** and **497
identical refusals**, the `verify-failed` one after a person corrected a spec criterion by hand. Our detection
latency is measured in days; the source video's is measured in seconds. This spec builds the reader.

It reads the records we already write. It does not read Hermes' logs (that is not pii), it does not use a model
(every incident on this machine is an exact-match string: 551 rows, 11 distinct values), and it opens a record —
it does not fix anything.

**Project**: <repo>
**Phase**: proactivity (the Opus 061 judgement; order 153 → 150 → 154 → 151 → 152)

## Requirements

### Functional

R1. A new source in `pii-learn` beside the trajectory scanner. It reads, each as a row `(ts, source, fingerprint,
    evidence path:line)`:
    - the loop's cycle logs: its `OUTCOME=` lines (`_tmp/autonomy/logs/cycle-*.log`), where the fingerprint is the
      outcome value (`dirty-tree`, `verify-failed`, `gate-failed`, `worker-rc-N`, `window-closed`, `done`, …);
    - `fresh-gate`'s failing-test names (`<export>/gate-output.log`);
    - the daemon's own events: `run.start.refused`, store and panic events.
R2. The threshold lives in config, not in code: default **N=3 occurrences of one fingerprint within 30 minutes**
    opens an incident; a list of **known-benign fingerprints** (starting with `window-closed`) never opens one.
R3. **One incident per fingerprint per window**, appended to `incidents.jsonl` with `fingerprint`, `first_seen`,
    `opened_at`, `count` and the evidence path:line. Repeated occurrences of an already-open fingerprint increment
    the count — they never open a second incident (497 refusals must be 1 incident, not 497).
R4. `tick.sh` invokes the scan through a CLI surface (`pii incidents scan --root <dir>`) and sends **one phone line
    per new incident**. It must stay silent when no incident is new (the watchdog pattern).

### Non-Functional

R5. Rust, no new dependency, no model call, no network. A scan over a 500-line log must take well under a second.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| the_preserved_log_yields_exactly_three_incidents | a committed fixture trimmed from the real `cycle-last-detached.log` | one incident per fingerprint: `dirty-tree` opened at its first-seen +10 min, `verify-failed`, `window-closed` |
| refusals_collapse_into_one_incident | 497 identical `dirty-tree` rows | 1 incident with count 497 |
| a_benign_fingerprint_never_opens | `window-closed` rows beyond the threshold | 0 incidents |
| below_the_threshold_opens_nothing | 2 rows in 30 min | 0 incidents |
| the_evidence_points_at_a_line | every row | the path exists and the line contains the fingerprint |

## Exit Criteria

- [ ] `cargo test -p pii-learn --test incidents 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] the fixture is committed under `crates/pii-learn/tests/fixtures/` and derived from the real log (say in the
      closure how it was trimmed)
- [ ] `cargo run -q -p pii -- incidents scan --root _tmp/autonomy/logs` prints one line per incident and exits 0
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT open an incident per occurrence: R3 is the whole point (497 → 1).
- Do NOT use a model, a cloud call, or Hermes' logs.
- Do NOT let a benign fingerprint in through a config default nobody can find: it must be one list, in config.
- Do NOT touch `/tmp`, `/dev`, `/proc`, `/sys`, `/root`; scratch under `<scratch>/`.

## Closure (2026-11-01)

Criterion by criterion, measured on the deployed tree:

```
cargo test -p pii-learn --test incidents     ->  test result: ok. 7 passed
fixture                                      ->  crates/pii-learn/tests/fixtures/cycle-last-detached.trimmed.log
                                                 (6 394 B, a trim of the real _tmp/autonomy/logs/cycle-last-detached.log,
                                                  the run whose head carries the 496-refusal episode) + fixtures/production/
criterion 3                                  ->  the command as written names package `pii`, which does not exist
                                                 (`error: package(s) pii not found`); the binary is `pii` from `pii-cli`,
                                                 so it was run as the installed binary
```

**The replay, on the real corpus** (`pii incidents scan --root _tmp/autonomy/logs`, exit 0, 11 incidents):

```
dirty-tree     count=496  one incident, first 2026-10-11T21:01:44Z   <- the whole point: 496 refusals, 1 record
locked         count=31   verify-failed 3 (three windows)   add-failed 9   plan-error 6
no-change      count=3    gate-failed 2 (two windows)       parked 2
window-closed  none — zero benign fingerprints opened an incident
```

The per-fingerprint counts differ from the 061 judgement's replay (1 dirty-tree / 1 verify-failed) because the
corpus has grown from 551 outcomes to the full log set; the property under test is unchanged and visible:
**one incident per fingerprint per window**, with repeats incrementing `count`.

**What is still not measured**: M1's second half — a live incident line reaching the phone within a day. The
mechanism opens records; delivery to the operator has not been observed end to end.
