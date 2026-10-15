# Spec 160: the reader reads the real log (Phase B — production detection, measured on production)

**Project**: pi-improved · **Milestone**: 063 Phase B — *"the reader must read the real log; time-to-notice on production"*, ranked second of five because everything the lane claims rests on it.
**Why now, with the measurement**: 156 shipped the runtime reader and its tests pass on fixtures. On the REAL daemon log the review measured the opposite: **it sees 0 of 5 starts**, and on the audit log it is **false 5 of 5**. So today, detection on production is either blind or wrong, and the "detection latency" the whole lane promises has never been measured against a single real event.

## Requirement
The runtime reader (156) must judge the daemon's own records as they are actually written:

- **starts are seen**: a supervised unit restarts the daemon; each start, however it appears in `pii.log` (including after rotation at `--log-rotate-bytes`), must produce exactly one incident (or exactly one start record, per the rule 156 chose — stated in the code, not implied)
- **the audit log is not evidence of an incident**: the audit lines that today open 5 false incidents must open zero, and the reason must be a rule (what makes a line evidence), not a string that happens to be absent
- **time-to-notice is computed on the real log**: from the first evidence line to the incident, printed, for a named log file, with the count of starts considered
- **rotation is handled**: with `--log-keep-files 5`, an event in a rotated file is still seen; the reader must not silently read only the current file

## TDD contract
| Test | Input | Expected |
|---|---|---|
| the REAL log, copied | a copy of the daemon's own `pii.log` from production | every start present in it is reported; the count equals the number of starts the log contains (not zero) |
| the REAL audit log, copied | a copy of production's audit log | **0 incidents** — the false-positive regression, pinned |
| a rotated pair | current file + one rotated file, event only in the rotated one | the event is reported (rotation is not a blind spot) |
| a log with no events | an empty/clean copy | 0 incidents, exit 0 |
| the same input twice | any of the above | byte-identical output (a trend line depends on it) |

**The fixtures are COPIES OF THE PRODUCTION LOGS, not hand-written samples** — this is the whole point of the spec: today's failing behaviour is a fixture-versus-reality gap, and hand-written fixtures are what produced it.

## Exit criteria
- [ ] `cargo test -p pii-learn --test incidents_runtime_production` → rc=0
- [ ] `cargo run -q -p pii-cli -- incidents scan --source runtime --log <copy of production pii.log>` → prints a NON-ZERO number of start records, and the number matches a grep of the file
- [ ] the same command against a copy of the production audit log → prints 0 incidents
- [ ] the printed time-to-notice for the earliest start in that copy is a real duration (not zero, not absent), and the command says how many starts it considered
- [ ] `_docs/05-operations/production-progression.md` gains the measured line: starts seen N of N, false incidents 0, time-to-notice = X

## Guardrails
- Read-only over the daemon's logs; copy before parsing if a writer holds them (the single-writer rule)
- Do NOT edit the daemon's logging to make the reader simpler: the reader must fit the production format, not the reverse
- Do NOT weaken the clean-log control (156's `the_clean_control_opens_nothing`) — it must stay green
- Every fixture carries the command that produced it and the date, in a header comment of the test file

## Milestone link
`_docs/06-research/063-opus-next-phases-2026-10-15.md` Phase B · spec 156 (the reader this corrects) · `_docs/05-operations/production-progression.md`.
