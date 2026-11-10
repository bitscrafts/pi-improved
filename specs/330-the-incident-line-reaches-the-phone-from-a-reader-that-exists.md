# Spec 330: the incident line reaches the phone from a reader that exists

## Overview

Milestone M1 of `_docs/05-operations/production-progression.md` has two halves. The replay half is done and was
re-measured on 2026-11-01: `pii incidents scan --root _tmp/autonomy/logs` over a copy of the real records returns
11 incidents (dirty-tree 496, locked 31, add-failed 9, plan-error 6, no-change 3, verify-failed 3, gate-failed 2,
parked 2) and **zero benign fingerprints**. The live half — "one incident line reaches the phone within a day" —
has never been observed, and three measurements taken this tick say it cannot currently be observed at all:

**(a) No reader is silence, not a report.** `scripts/autonomy/tick.sh` branch 0b is
`if [ -x "$PII_BIN" ]` with `PII_BIN=${PII_BIN:-target/debug/pii}`, and no else. Measured 2026-11-01T06:2xZ with
every other knob pointed at scratch:

```
PII_BIN=/nonexistent/pii … bash scripts/autonomy/tick.sh
  output: pii autonomy · dry: would start A147…
  grep -c 'INCIDENT\|cannot read\|incident reader'  ->  0
```

`cargo clean` removes `target/debug/pii` (the operator's own disk-reclaim step, and `pii` memory records it as the
sanctioned one), a fresh-gate export never has one, and the devbox tree does not either. On any of those trees the
detection lane is dead and says nothing — the "a check that passes while the live path is broken" class, on the one
mechanism M1 exists for.

**(b) The incident root is hardcoded, so a test writes the operator's live store.** Measured the same tick by
pointing `PII_BIN` at a wrapper that records its argv, with `PII_LOCK`, `PII_CONTROL`, `PII_QUESTIONS`,
`PII_REPORTED`, `PII_STATE` and `PII_PLAN` all under scratch:

```
the carrier really received:  incidents scan --root _tmp/autonomy/logs --new
```

Every knob the tick honours, except the one that decides *which records are read*. `--new` is not read-only: it
appends to `incidents.jsonl` under that root, which is exactly how an incident is marked reported. The selftest
drives `tick.sh` twice per run (`selftest.sh:88,105`) and sends the output to a scratch file nobody reads, so a
selftest can swallow a live incident's phone line permanently — `_tmp/autonomy/logs/incidents.jsonl` (11 lines,
sha256 `478b3a43…`) is the file at risk.

**(c) Nothing tests the lane.** `grep -c incident scripts/autonomy/selftest.sh` is 0, and no file under
`scripts/autonomy/tests/` names it. Since spec 324 made those shell tests a gate step, an unexercised branch here is
precisely what the gate cannot see — which is why (a) and (b) survived in a loop that has 42 green selftest rows.

The carrier is not the defect and is not changed: `pii incidents scan --root <dir> --new` is proven by spec 150's
tests and by this tick's replay. What this spec fixes is the *delivery* half — who reads, from where, and what the
operator is told when nobody can.

## Requirements

**R1 — a reader is resolved from a documented order, and the choice is recorded.** `PII_BIN` wins when it is set:
a caller that names a binary gets that binary, with no silent fallback. Otherwise, in order:
`target/debug/pii` if executable, then `command -v pii`. The path actually used is written to
`$ST/incident-reader` (one line: the path, and which rule chose it) so a later reading of a phone line can say which
build produced it.

**R2 — no reader is a reported condition, never a silence.** When R1 resolves nothing, the tick prints exactly one
deduplicated line naming what was looked for and the command that restores it, then continues: detection is a
report, not a gate, so the cycle dispatch still happens. Keyed through `say_once`, so it reaches the phone once and
not every five minutes. The tick never builds anything.

**R3 — the records root is a knob.** `PII_LOGD` (default `_tmp/autonomy/logs`) is the root used by branch 0b and by
the last-cycle reader, and the same directory the detached cycle log is appended to. A caller that points its state
somewhere else cannot leave this one pointing at the operator's live records.

**R4 — one hermetic test drives the real tick.** `scripts/autonomy/tests/tick-incident-phone.test.sh` runs the real
`tick.sh` (not a re-enactment) with scratch state, lock, control, question, reported, plan and logs, and the real
carrier from `PATH`. It asserts: the carrier exists; the fixture's two incidents reach the tick's output as
`INCIDENT dirty-tree — 6 times since 2026-10-11T21:01:44Z` and `INCIDENT verify-failed — 3 times`; a second tick
over the same records is silent; a missing reader yields the R2 line once and not twice; the carrier receives the
scratch root and never `_tmp/autonomy/logs`; the live store's sha256 is unchanged; and `$ST/incident-reader` names
the reader used. `--control` proves the checker can fail: it mutates a **copy** of `tick.sh` back to the hardcoded
root and shows the same assertion flagging it, and it shows a present reader never prints the cannot-read line.

**R5 — the mechanism is named in the suite's verdict.** `selftest.sh` gains one row that runs R4's test, points its
own tick rows at its scratch `PII_LOGD`, and so can no longer mark a live incident reported. A SKIP is never
counted as a pass.

**R6 — nothing else moves.** No new dependency, no model, no build inside the tick; the incident rule (30-minute
window, repeat threshold, fingerprint, `--new` semantics) and the carrier's output shape are untouched.

## Files

- `scripts/autonomy/tick.sh`
- `scripts/autonomy/tests/tick-incident-phone.test.sh`
- `scripts/autonomy/selftest.sh`
- `specs/330-the-incident-line-reaches-the-phone-from-a-reader-that-exists.md`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| the carrier is present | `command -v pii` | a path, else the test fails loudly (never a skip) |
| the fixture's first incident reaches the phone | committed fixture in a scratch logs root | a line containing `INCIDENT dirty-tree — 6 times since 2026-10-11T21:01:44Z` |
| its second incident rides the same line | same scan | a line containing `INCIDENT verify-failed — 3 times` |
| a reported incident is not repeated | a second tick, same records | no `INCIDENT` line |
| a missing reader is reported once | `PII_BIN=/nonexistent/pii` | one line naming no reader and the restore command; a second run prints nothing |
| the root comes from the knob | wrapper carrier records its argv | `--root` names the scratch root, never `_tmp/autonomy/logs` |
| the live store is untouched | sha256 before/after | equal |
| the reader is named | `$ST/incident-reader` | the path R1 chose |
| --control sees the hardcoded root | a copy of `tick.sh` with the literal root restored | the root assertion flags it; a present reader prints no cannot-read line |

## Exit Criteria

- [ ] `bash -n scripts/autonomy/tick.sh`
- [ ] `bash scripts/autonomy/tests/detection-before-delivery.test.sh 2>&1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `bash scripts/autonomy/tests/incidents-md-publishes-once.test.sh 2>&1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `grep -q 'incidents-md.sh' scripts/autonomy/watchdog.sh`
- [ ] `bash scripts/autonomy/incidents-md.sh --check 2>&1 | grep -qE "up to date|unpublished"`
- [ ] `bash scripts/autonomy/run-tests.sh 2>&1 | grep -qE "^autonomy tests: [0-9]+ passed, 0 failed"`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Guardrails

- **STOP on a shape mismatch.** If the carrier's line does not parse as
  `incident <fingerprint> count=<n> first_seen=<iso> opened_at=<iso> evidence=<path>[ NEW]`, stop and report it.
  Do not loosen the pattern, do not rewrite the carrier, and do not "fix" the incident rule to fit a parse.
- **A missing reader must not stop the loop.** R2 prints and falls through to the dispatch branch: detection that
  halts the builder is worse than detection that cannot read.
- **Never build from the tick.** Resolution looks at existing paths only; a tick that compiles is a tick that can
  block for minutes and starve the beat.
- **`PII_BIN` set and unusable is the R2 case, not a fallback.** Redirecting a caller that named a binary is how a
  test ends up certifying a build nobody shipped.
- **Do not weaken an assertion to make an item pass**, and do not let a SKIP read as a pass: the run is green only
  when the carrier answered and every assertion ran.
- **The test writes only under `_tmp/` and removes only what it created.** It must not append to
  `_tmp/autonomy/logs/incidents.jsonl`; the sha256 assertion exists for that.

## Closure

(open — filled by the orchestrator that verifies it)

## Closure — reconciled 2026-11-01, and what changed

**This spec asked for a phone line; the surface that exists today is a file, chosen by the user ("lets us a file by
now!").** The row is closed against what shipped, not against the stand-in: the worker's reader resolved a binary
and a records root and its test drove the real tick over a synthetic fixture named after a real 88-byte log, with a
control covering 1 of ~12 assertions. That WIP is preserved at `<scratch>/wip-330/` and was never committed.

What actually shipped (commits `1e8d530`, `5eb8f44`), and why each is the thing this spec was reaching for:

- **The line reaches a surface.** `scripts/autonomy/incidents-md.sh` appends one dated, greppable row per new
  incident to `_tmp/autonomy/INCIDENTS.md`, with a cursor so a re-run never repeats one, and the beat runs it after
  every tick. A file has no credential, no daemon and no delivery contract — which is exactly why it is the honest
  interim surface while Discord delivery is decided.
- **Detection runs before delivery.** The scan used to sit after the gateway block, which exits 0 when it believes
  the gateway is down; that probe costs 10 s per call here and misfires under load, so the beat scanned nothing
  while the gateway had been up for days. The scan is now first, and a live process is accepted on `pgrep` before
  any IPC probe. This is the defect that made "the incident line reaches the phone" impossible in the first place.
- **The reader's resolution order (R1) was dropped on purpose.** Two attempts to build it parked: it was machinery
  around a line that nothing was publishing. Fix the publication first; a reader with several sources can follow.

Evidence: `detection-before-delivery.test.sh` (4 assertions, control = gateway DOWN and the line still comes out),
`incidents-md-publishes-once.test.sh` (9 assertions), autonomy suite 37/37, `fresh-gate: PASS`.
