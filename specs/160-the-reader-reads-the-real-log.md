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

## Closure
- **2026-10-16, orchestrator: the `gate-failed` rows this item carried were the GATE, not the worker.** The fresh-gate
  that stopped it executed `_tmp/gate-target/debug/deps/launcher-b20b8ddc02b01e72`, whose dependency file records
  `env-dep:CARGO_MANIFEST_DIR=/projects/_tmp/fresh-gate-72466/crates/pii-evolve` — the export of a *previous* run
  (the 01:39 fresh-gate verification), whose `crates/pii-evolve/src/` no longer exists. The three tests that read
  files relative to `CARGO_MANIFEST_DIR` (`driver_spawns_no_subprocess`, `no_raw_dial_remains_in_pii_evolve`,
  `launcher_dials_through_the_uds_client`) answered `read driver.rs: NotFound`, and the gate went red on a tree that
  is green: **the same five tests pass 5/5 when built for THIS tree, measured twice** (default target dir; and with
  the gate's own `CARGO_TARGET_DIR=_tmp/gate-target`). A shared warm cache plus per-run throwaway export paths is
  what lets an artefact carry a source path that has since been deleted. Worth a mechanism fix of its own.
- **The second red was environmental and is now fixed.** The gate's `clippy (llamacpp)` step builds
  `llama-cpp-sys-2` from source, which needs `cmake`, `make`, a `libclang` that bindgen can load, and the OpenSSL
  headers; the box had lost all of them (measured: `which cmake gmake` → empty; bindgen `Unable to find libclang`;
  `is cmake not installed?`; `gmake: ... openssl/err.h file not found`). Restored by the orchestrator on 2026-10-16:
  `cmake 3.31.6`, `make 4.4.1`, `libclang-19-dev`, `libssl-dev`, `pkg-config`, plus `cargo-llvm-cov 0.9.1` and the
  `llvm-tools` component that the coverage-floor step's own comment demands ("their absence is a FAIL, not a skip").
  **Measured after the restore:** the step's command verbatim,
  `CARGO_TARGET_DIR=target/llamacpp-gate cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings`
  → **rc=0** (it had been rc=101 in every run since 2026-10-15 13:35, which is what parked this item and item A22).
- **Criteria, run by the orchestrator on the committed tree (`a8403f0`), all rc=0**: `cargo test -p pii-learn --test
  incidents_runtime_production`; the scan of the production `pii.log` copy → `starts_considered=1 incidents=0
  files=1`; the scan of the audit-log copy → `starts_considered=0 incidents=0`; the printed
  `time_to_notice_ms=22`; and `production-progression.md` carrying "starts seen 1 of 1". The plan row A27 carried
  only the first of these five; it now carries all five, concretised against the spec's own fixture copies (the row
  had dropped four criteria — the class the plan header warns about).
- **Still owed:** one green fresh-gate on this tree, and nothing else. The implementation and its fixtures are
  committed; no worker dispatch is owed.
- **2026-10-16 12:15Z, orchestrator: the four rows that parked it are all `gate-failed`, none `verify-failed`.**
  Measured from `_tmp/autonomy/logs/cycle-last-detached.log`: A27 answered `OUTCOME=gate-failed` at 02:10, 02:53,
  07:26 and 08:24, and was parked by the attempt cap. Its own criteria were never the reason, so the four sessions
  on disk remain the right resume handles for when the gate is green — re-queue the row, do not re-derive it.
- **The red it now carries is a third cause, new and neither of the two above:** `boss_watch_reaches_detect_after_resume`
  stops growing its journal at a fixed entry under CPU load (measured twice, the second with a 32-hog control;
  `_docs/05-operations/DEFECT-2026-10-16-boss-watch-stalls-under-load.md`), which a paid review was answering at
  12:14Z. The cheap criterion was re-run green today: `_docs/05-operations/production-progression.md` carries
  `starts seen 1 of 1`. The two `cargo test`/`cargo run` criteria need a build and were deliberately not run while
  the review was timing gate builds on an otherwise idle host. "Still owed" is unchanged in kind and now has a
  name: one green fresh-gate on this tree cannot come before the boss-watch fix.
