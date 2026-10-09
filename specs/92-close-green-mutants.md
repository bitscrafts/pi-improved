# Spec: 92 — close the five green mutants (spec 91 findings 1-5)

**Project**: /projects/pi-improved
**Phase**: 045 follow-through. Spec 91 measured which guards actually bite; five mutants stayed GREEN.
Each is a test-quality gap (not a production bug) and each gets a test that turns red when the mutant is
re-applied. The mutation log is the source of truth: `_docs/06-research/046-mutation-verification-batch-cd-2026-10-09.md`.

## Requirements
R1 (finding 1 — no `get_graph` seam): add a store seam that makes `get_graph` fail (follow the existing
    `set_fail_get_run_for_test` shape in `crates/pii-store/src/testing.rs`), and a test asserting that a
    resume whose GRAPH BODY is unreadable/unparsable is REFUSED (returns `Err`, registers no run). The
    existing `resume_fail_closed_on_graph_lookup_fault` faults `get_run` and asserts only `is_err()` — it
    must be renamed or split so it says which of the two faults it covers, and the new test must fail
    under spec 91's R2a mutant (graph body falling back to unlimited).
R2 (finding 2 — seeded limits are already unlimited): `resume_fail_closed_on_*`'s `seed_real_run`
    (runs.rs:568) must seed FINITE limits, and a test must assert the resumed run carries the RECOVERED
    limits (not `u64::MAX`). It must fail under spec 91's R2b mutant.
R3 (finding 3 — `recover_finished` wiring unpinned): a test that drives the REAL `reactive_loop` (flood
    the bus past capacity, as spec 79's test does for the timer case) with a cron run whose terminal
    `RunFinished` is lost, and asserts the cron chain was re-armed. It must fail when the
    `lag::recover_finished` call is removed from the `Lagged` arm (spec 91 R3b).
R4 (finding 4 — `store_op` argument swaps undetected): a unit test that pins the FULL rendered message of
    at least three `store_op` sites, including one whose resource is a table name (`"begin_write"`/`"db"`,
    `"read"`/`"run_counter"`, `"open"`/`"graph_body"`), so swapping op and resource at those sites fails.
    Must fail under spec 91's R5b mutants.
R5 (finding 5 — prefix-only assertion): `open_failure_renders_structured_store_op` (lib.rs:645) must pin
    the resource and the detail, not just `starts_with("store op open on ")`. State in a comment why the
    resource is the full path at that site.

## Exit Criteria
- [ ] every one of the five mutants from `046-…md` (R2a, R2b, R3b, R5b-1..4, and the prefix-only shape) re-applied ONE AT A TIME turns at least one test RED; the log in `/projects/_tmp/spec92/MUTATION-LOG.md` records the test name and assertion verbatim per mutant
- [ ] baseline: those same tests are GREEN on HEAD with no mutant applied
- [ ] `cargo test -p pii-server`, `cargo test -p pii-store`, `cargo test -p pii-graph` all pass
- [ ] `bash quality-gate.sh` exits 0 and `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0
- [ ] no file passes the lint-files warn threshold (report `lint-files` output)
- [ ] the report lists every file touched and confirms `specs/` was not edited

## Guardrails
- TESTS AND TEST SEAMS ONLY. Do NOT change behaviour of `spawn_resume`, `store_op`, `reactive_loop`,
  `recover_finished` or the `Lagged` arm to make a test pass — if a mutant cannot be caught without a
  production change, report that instead and leave it as a finding for a separate spec.
- Do NOT weaken an existing test (e.g. do not relax an assertion or add `#[ignore]`).
- Do NOT re-litigate spec 91's already-RED rows; they are proven.
- Scratch under `/projects/_tmp/`; delete your own exports (one ≈ 8G).
- Do NOT commit; leave the tree for the orchestrator.
