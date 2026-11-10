# Spec 156: pii reads its own runtime logs (M6 — the missing production milestone)

**Project**: <repo> · **Milestone**: M6 — the step that makes the operator's demand real: a process dies and the SYSTEM says so.
**Why now**: M1–M4 are implemented and committed (150 detection from our own records, 154 evidence before the first node, 151 an incident to a proven fix on a branch, 152 the deploy card). M5 is the person's two minutes. M6 is the half that does not exist: the reader only knows the BUILDER LOOP's records; the deployed daemon's runtime produces records nobody reads.

## Requirement
`pii incidents scan` gains a source for the DAEMON's own runtime records, with no model involved:
- an incident opens when the daemon records an unexpected restart or a start that did not reach ready
- an incident opens when a run ends without its terminal event, or its evidence file is older than the configured window
- an incident opens when the same refusal repeats beyond the configured count inside the window
- ONE line per incident: fingerprint, count, first and last timestamp, and the source file it was read from

## TDD contract
| Test | Input | Expected |
|---|---|---|
| a preserved daemon log with one stalled run | fixture log | exactly 1 incident, fingerprint naming the run id |
| a control log with no anomaly | clean fixture | **0 incidents** (the control that makes the above mean anything) |
| the same refusal repeated 5x in the window | fixture log | 1 incident with count=5, not 5 incidents |
| the source log is read, never written | fixture + mtime/hash before and after | hash unchanged, no new file beside it |

## Exit criteria
- [ ] `cargo test -p pii-learn --test incidents_runtime` → rc=0
- [ ] `cargo run -q -p pii-cli -- incidents scan --source runtime --log tests/fixtures/runtime-stalled.jsonl` → prints exactly one incident line, and `--source runtime --log tests/fixtures/runtime-clean.jsonl` prints none
- [ ] the incident line carries the fingerprint, the count, and the file it came from (grep for all three)
- [ ] detection latency is computed and printed: first event → incident, against the configured window

## Guardrails
- Do NOT read Hermes' logs inside pii (061: out of scope)
- Do NOT add a model or a judge node (061: the records are exact strings today)
- Do NOT mutate, rotate or truncate the daemon's log; copy before parse if a single writer holds it
- Do NOT widen 150's builder-loop reader; this is a second source, selected explicitly

## Milestone link
`_docs/05-operations/production-progression.md` M6; acceptance is the same shape as M1: replay a preserved log ⇒ an exact count of incidents, and one live incident line reaching the operator's phone within a day.
