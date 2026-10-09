# Spec: 105 — P1: make it operable (backup, upgrade, metrics, runbook)

**Project**: /projects/pi-improved
**Phase**: P1 of the road to production (`_docs/06-research/052-production-readiness-2026-10-09.md` §8).
**Goal**: the operator can recover, and can see the system's state without reading logs.

## Measured facts this spec starts from
- Spec 99 landed health identity + `pii health --json`; it exposed a real gap: the **main store** reports
  `format_version: 1`, but the **memory store reports `format_version: null`** — so a version refusal
  covers one file, not the stores the daemon actually opens.
- Stores on disk today: `store.redb`, `memory.redb`, `approval.redb`, `heal.redb` (1,056,768 bytes each in
  the smoke run). Tables in the main store: `META`, `GRAPH_BODY`, `RUN`, `RUN_JOURNAL`, `RUN_SNAPSHOT`.
- There is no backup/restore path, no migration step, no metrics endpoint and no runbook.

## Requirements
R1. **Stamp a format version into EVERY redb the daemon opens** (main, memory, approvals, heal): write it on
    create, read it on open, and **refuse to open a store whose version is NEWER than this build** with a
    clear message naming the file, the found version and the supported one. Surface all of them in
    `pii health --json` (the `null` must be gone).
R2. **Prove the refusal with a deliberately bumped version**: set a store's version above the supported one
    (a test-only helper or a direct write), then show the open fails with the clear message and nothing is
    corrupted — and paste the message. Control: the un-bumped store still opens.
R3. **Backup → wipe → restore round-trip**, documented and exercised in a scratch dir with real stores:
    - a `pii backup <dest>`-style command (name it as you see fit) that copies the stores **and** their
      metadata consistently (a redb copy must not tear: say how you guarantee that — e.g. a read
      transaction snapshot or `redb`'s own copy, and prove it);
    - a restore path that refuses to overwrite a non-empty store directory without an explicit flag;
    - the round-trip proof: create a run/approval, back up, move the originals aside, restore, and show
      `pii health --json` counts and the run read back identical (paste both).
R4. **A metrics surface**: one endpoint or one command that emits the counters/gauges from the contract
    (runs, sessions, approvals pending, journal events, memory docs, store bytes, event totals, plus any
    histogram the contract already exposes) in a documented, stable shape. Point at the exact names it
    uses (they must be `pii_observe::names` consts).
R5. **A runbook**: short, operational, no marketing — start/stop, where the store and logs live, how to
    read health, how to back up and restore, how to upgrade (including the version refusal and what to do
    about it), and how to diagnose a wedged daemon (which bounded shutdown line to expect: spec 95 logs
    `shutdown drain exceeded …`).
R6. **Structured logging documented**: the log file + rotation from spec 104 appears in the runbook with
    the exact config keys.

## Exit Criteria
- [ ] every store reports a real `format_version` in `pii health --json` (paste the JSON; the `null` is gone)
- [ ] the bumped-version refusal is pasted with its message, and the control (un-bumped store opens) is pasted
- [ ] the backup → wipe → restore round-trip is pasted end to end (commands + counts + the run read back)
- [ ] the restore guard is pasted (refuses a non-empty target without the explicit flag)
- [ ] the metrics surface is pasted with its real output and the name consts it emits
- [ ] the runbook exists, is checked in, and names every command it tells the operator to run
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate
      --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report lists every file changed and every command an operator needs

## Guardrails
- **Never destroy a store**: the round-trip must MOVE the originals aside (`mv`, never `rm`), and the test
  must create its own scratch. Restoring over a live store requires the explicit flag.
- A version refusal must refuse — no silent fallback to "open it anyway".
- Metrics/logs must never carry secrets or env values.
- No new raw prints beyond the file's budget; emit through `pii-observe`.
- Do not leave a daemon running (kill what you start).
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.
