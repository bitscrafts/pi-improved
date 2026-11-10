# 184 — the runtime's re-arm waits for the next occurrence, not fifteen seconds

## Overview

MEASURED 2026-10-18T06:20–06:26Z by the orchestrator, on a clean tree (`git status --porcelain` empty,
HEAD `136b23e`):

```
cargo test -p pii-server --test cron_runtime
  thread 'a_finished_cron_run_is_started_again_inside_the_runtime' panicked at
    crates/pii-server/tests/cron_runtime.rs:92:5:
    the cron was started 1 time(s); the re-arm never ran a second
  test result: FAILED. 0 passed; 1 failed   (15.1 s)      — run twice, same result
cargo test -p pii-server --test timer_no_refire
  test result: ok. 1 passed; 0 failed                     (6.3 s)
```

The test's graph is one `timer` node with `cron = "* * * * *"` and `[limits] max_steps = 0`, so the first
firing ends at once and the second run must come from the re-arm. Its ceiling is **15 s**.

That ceiling encodes the behaviour spec 179 removed. `03e729a` (179's real fix, 2026-10-17 23:04Z) made a run
that parks on its timer *stay tracked*:

```rust
// crates/pii-server/src/daemon/cron.rs:71-78 (moved there by 136b23e from daemon/mod.rs)
// Spec 179: a run parked on its timer journals `Finished{Suspended}`; it is not over, and
// re-arming here started a new run every CRON_MIN_REARM. Stay tracked until it truly ends.
let parked = matches!(terminal, WireTerminal::Suspended { .. });
let cron_name = if parked { None } else { ctx.cron_runs.lock().remove(run_id) };
```

So a `Suspended` run is not re-armed. A second run can only start once the first one's occurrence fires — up to
one minute later for `* * * * *` — and then truly ends. Before `03e729a` the re-arm fired on `Suspended` every
`CRON_MIN_REARM` (1 s), which is exactly the 516-runs-in-four-minutes defect of 179. **The test passed because it
asserted the defect.**

This red is at HEAD and no loop gate saw it:

- the per-item gate is `quality-gate.sh --routine` = `gate-steps-quick.txt` (**fmt, clippy, build**), no tests;
- A50/183's own verify was two named commands (`cargo test -p pii-server --lib`, `--test timer_no_refire`), and
  both still pass;
- the suite step that owns this file is `test (lib/bins/tests)` in `gate-steps.txt`, the release boundary — so
  `quality-gate.sh --fresh`, the gate a deploy card must quote, is RED at HEAD.

Measured for the item that should follow this one (the routine gate gaining a test step): the full suite in this
tree, warm cache `_tmp/gate-target/test-gate`, was **293 s cold** (`-j 2 --workspace --lib --bins --tests`,
despite the name the cache was cold for this source root) and one crate scoped (`-p pii-server --lib --bins`)
**96 s**. Recorded in `_docs/05-operations/open-items.md` §9b.

**Project**: <repo>

## Requirements

1. **The second start is waited for, not timed at 15 s.** The test's ceiling must span one full occurrence of its
   own cron (`* * * * *` = 60 s) plus margin — at least 75 s — and it keeps polling the store for the count
   (no fixed `sleep` standing in for the event).
2. **The contract 178 wrote still holds:** a second run is started inside a Tokio runtime, with no *"there is no
   reactor running"* panic; `started >= 2` stays the assertion.
3. **179's rule is asserted at the runtime level too:** once the second run exists, a further 6 s (six times the
   1 s minimum re-arm) must not start a third.
4. **The neighbouring contracts stay green:** `cargo test -p pii-server --lib` (129 passed, 0 failed at HEAD) and
   `cargo test -p pii-server --test timer_no_refire` (1 passed, 0 failed) in the same tree.
5. **No product change.** `cron.rs`'s parked rule stays exactly as it is; the deliverable of this spec is the
   test's window and its second assertion.

## TDD Contract

- **RED (reproduce it first, and record the counts):** `cargo test -p pii-server --test cron_runtime` → 1 failed,
  `the cron was started 1 time(s); the re-arm never ran a second`, about 15 s. Do not change the file before you
  have this output.
- **GREEN:** the same command rc=0 within ~75 s, and `cargo test -p pii-server --test timer_no_refire` still
  reports 1 passed in its six-second window, in the same tree.
- **The false green to refuse:** making the test pass by re-arming a `Suspended` run (`cron.rs:74-78` reverted to
  `ctx.cron_runs.lock().remove(run_id)` for every terminal) is NOT green — that is 179's defect, and
  `timer_no_refire` plus `cargo test -p pii-server --lib` must stay green in the same tree.
- **The other false green:** deleting the test, lowering `started >= 2`, or loosening it to `>= 1`.

## Exit Criteria

- [ ] `cargo test -p pii-server --test cron_runtime`
- [ ] `cargo test -p pii-server --test timer_no_refire`
- [ ] `cargo test -p pii-server --lib`

## Guardrails

- **Do NOT** weaken `started >= 2`, rename the test, or delete it. Its subject — a re-arm that runs inside a
  runtime instead of panicking — is spec 178's, and it stays.
- **Do NOT** revert 179: the parked rule in `cron.rs` is a measured fix (516 runs in four minutes), not an
  optimisation to undo.
- **Do NOT** raise `CRON_MIN_REARM` or any interval to make a third run disappear; the second assertion is about
  how many runs one occurrence starts, not how fast a re-arm is attempted.
- **Do NOT** touch `specs/`.
- **STOP and report** if the second start does not appear within one full occurrence plus margin: then the
  product genuinely never re-arms a run whose occurrence has passed, the defect is in `cron.rs`, and lengthening
  a test ceiling is the wrong instrument. Report the counts *and* the status of the single run (Suspended,
  Running, Succeeded) instead of picking a fix quietly.

## Closure

(to be filled by the orchestrator: the RED reproduced twice with its counts, the GREEN run by hand, the 179
control, and the mutation — reverting `cron.rs:74-78` to re-arm every terminal must redden `timer_no_refire`,
restored byte-identical afterwards with its `sha256sum` before and after.)
