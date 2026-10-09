# Spec: 103 — the trajectory substrate: the first brick of the self-improvement pillar

**Project**: /projects/pi-improved
**Phase**: P0/P1, observability — user directive (2026-10-09): *"observability is the basis of the self
improvement loop pilar"*. Rationale: `_docs/06-research/053-…md` §8. This spec extends the contract with
CORRELATION and adds the durable trajectory sink; it does NOT build skill extraction.

## Context
Pillar 4 (`001-pii-vision.md`) is skills + procedural templates + trajectory logging for later
fine-tuning, with `outcome.verifier`, `user_signal`, `escalation` and mandatory `redaction` — and it is
UNBUILT (052 §5.1). Every element of it is an observation, so the cheapest route is to make the existing
observability stream *sufficient* as a trajectory corpus. Spec 97 landed the contract
(`Event { subsystem, level, kind, fields, span }`) but **without run/session correlation**, which a
trajectory requires.

Already landed by the earlier parts of this lane (do NOT redo; use it): the contract crate `pii-observe`
(97), the failing gate step `lint-observability` with a **complete** name registry (98, R1e), health
identity + `pii health --json` (99), the adoption sweep — fifteen crates emit, each with a named-event
test (100, 101), and the **bounded** exemption registry + per-file print budget (102). So: a new
`pub const` in `names.rs` MUST be listed in `ALL` or the gate goes red, and a new raw print MUST fit the
file's budget or the gate goes red. Work with that, not around it.

## Requirements
R1. **Extend `pii-observe`** with correlation, outcome and redaction (additive, serde-stable):
    - `Event` gains `run: Option<RunId>`, `session: Option<SessionId>` (string newtypes), keeping `span`.
      Old payloads must still deserialize (a test pins a pre-103 JSON line).
    - An explicit outcome type mirroring the engine's terminal: `Outcome { verdict, reason, detail }`
      with `Verdict { Success, Failure, Cancelled, Suspended }`, carried by a dedicated event kind
      (`run.outcome`) — the record must carry the terminal **verbatim**, not a re-derived summary.
    - `Redaction` — a policy trait (`fn redact(&self, field: &str, value: &str) -> Redaction`) plus a
      `Redaction::{Keep, Drop, Replace(String)}` result and a `NoRedaction` default.
    - Every new metric/event name goes in `names.rs` **and** in `ALL` (the lint enforces completeness).
R2. **The trajectory sink** (types in `pii-observe`; say where you put the file-writing implementation and
    why):
    - append-only, one JSON object per line, one file per run at a caller-supplied root
      (`<root>/trajectories/<run_id>.jsonl`), created on the first event of that run;
    - takes an `Arc<dyn Redaction>` and applies it to every field **at the sink** (never at the call
      site) — including `fields` keys and the outcome detail;
    - never deletes, truncates or rewrites a trajectory; a write failure is reported as an observability
      event and never panics or aborts the run;
    - bounded: a per-run cap (bytes or events) with a `trajectory.truncated` event when hit, so a
      runaway run cannot fill the disk.
R3. **Wire it** so a real run produces a trajectory today: `pii -p` and `pii graph run` (and the daemon's
    run engine) attach the sink, honouring `PII_TRAJECTORY_DIR` (default OFF) plus an optional
    `trajectory_dir` in the daemon config. Existing behaviour and tests must be byte-identical when unset.
R4. **Prove it end-to-end, with the numbers**: run a real
    `pii -p "…" --model ollama:<a small local model>` with `PII_TRAJECTORY_DIR` set, and paste:
    - the resulting file path and `wc -l`;
    - the `run.outcome` line verbatim;
    - the line count WITH and WITHOUT redaction exercised (a test that redacts a field, showing the sink
      dropped/replaced it while the original event was unaffected);
    - a test asserting a FAILED run records `Verdict::Failure` — **a failed run must never record
      success** (the laya lesson: a metric that swallowed an exit code produced a 70× fake win).
R5. **A negative case per new measure (mandatory)**: for the outcome event and the truncation event, a
    test showing the event does NOT appear when the thing did not happen (a successful run emits no
    failure outcome; a short run emits no truncation; the sink OFF writes nothing).
R6. Do NOT build skill extraction, distillation, or any learning loop in this spec — this spec ends at
    "trajectories are on disk, correlated, redacted, with verbatim outcomes". Inferring from them is a
    later spec.

## Exit Criteria
- [ ] `cargo test -p pii-observe` exits 0, including: serde round-trip of the extended `Event`, a
      **pre-103 payload still deserializes**, the redaction matrix (Keep/Drop/Replace), the truncation
      cap, and the "failed run records Failure" case
- [ ] an end-to-end run with `PII_TRAJECTORY_DIR` set produces `<root>/trajectories/<run_id>.jsonl`; paste
      the path, `wc -l`, and the `run.outcome` line
- [ ] a run WITHOUT the env var produces no trajectory file and the stdout ndjson stream is unchanged
      (paste both)
- [ ] with redaction configured, the chosen field is absent/replaced in the trajectory while the same
      field is intact in the stdout stream (paste both)
- [ ] the negative cases (R5) are pasted: no failure outcome on a successful run, no truncation on a
      short run, nothing written with the sink off
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate
      --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists every file changed and names the exact env var and config key that enable the sink

## Guardrails
- REDACTION AT THE SINK, never at the call site. A policy that can be forgotten is not a policy.
- The trajectory root must default OFF; no existing test may start writing files.
- Never delete, truncate or rewrite a trajectory file — append only; report failures as events.
- Do NOT put secret material into the contract types, and do NOT log env values or tokens.
- Do NOT change the wire types in `pii-proto` in this spec.
- Do NOT edit any file under `specs/`.
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
**Landed.** The sink is off by default (`PII_TRAJECTORY_DIR` or `[core] trajectory_dir` enables it;
`PII_TRAJECTORY_REDACT=a,b` drops named fields at the sink). `Event`/`OwnedEvent` gained `run`/`session`;
new `outcome` and `redaction` modules plus `trajectory.rs`; names `RUN_JOURNAL`, `RUN_OUTCOME`,
`TRAJECTORY_TRUNCATED`, `TRAJECTORY_WRITE_FAILED` all in `ALL` (the completeness lint would have caught a
miss). Worker notes a process deviation: tests were written alongside the code, so there was no RED first;
it compensated with a mutation (flipping the Failure→Success mapping turned two tests RED, restored green).

**Reproduced by the orchestrator (my own runs, not the worker's pastes):**
```
ON   PII_TRAJECTORY_DIR=<scratch> pii -p "reply with the single word: pong" --json --model ollama:granite4.2:3b
     → rc=0; <root>/trajectories/r000000000001.jsonl, 9 lines
     → run.outcome: {"fields":{"detail":{"files_changed":[],"model":null,"text":"pong","tokens_in":928,
        "tokens_out":49,"turns":1},"reason":"terminal","verdict":"success"},"kind":"run.outcome",
        "level":"info","run":"r000000000001","session":"s1","span":null,"subsystem":"graph"}
REDACTION  PII_TRAJECTORY_REDACT=text → rc=0; '"text"' in the trajectory: 0 occurrences;
     in the same run's stdout ndjson: 4 occurrences. The stream is unaffected; the SINK redacted.
OFF  same run, no env var → rc=0, no trajectories directory created at all.
FAILED RUN  cargo test -p pii-graph --test trajectory → 2 passed:
     failed_run_records_failure_with_the_engine_error_verbatim
     successful_run_records_success_and_no_failure_outcome   ← the negative case, per the laya lesson
```
Gates: `quality-gate.sh` ALL PASS · `fresh-gate --worktree` PASS · clippy `-D warnings` and `fmt --check` 0.
