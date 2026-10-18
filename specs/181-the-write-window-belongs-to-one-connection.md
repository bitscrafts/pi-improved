# 181 — the write window belongs to one connection

## Overview
MEASURED 2026-10-18 by the orchestrator, mutating spec 180's fix: the per-connection byte window is **proven on
the read path and unproven on the write path**. Restoring cumulative accounting on the write side
(`registry.bytes_out[*total] > grant.max_bytes_out` instead of the connection's own `sent`) leaves all three
tests in `crates/pii-net/tests/budget_survives_a_day.rs` **green** — no test in that file moves more than about a
kilobyte through one connection's `write_all`, and the five-thousand-poll test's volume is charged to
`max_bytes_in`. A regression that re-shared the out-window across the connections of one grant would ship green,
which is the same failure shape `180` just closed for reads (`request 487 refused: capability denied: net: byte
budget exhausted`).

**Project**: /projects/pi-improved

## Requirements
1. Add a test to `crates/pii-net/tests/budget_survives_a_day.rs`, named exactly
   `a_write_window_is_spent_per_connection`, that opens **two** connections on **one** grant, writes ~700 KB on
   each through `Conn::write_all` under a 1 MB `max_bytes_out`, and requires both to succeed. A counter shared
   across the grant refuses the second connection, so the test is red on that accounting and green on the
   committed one.
2. The three existing tests keep their behaviour: the oversized response is still refused, the concurrent-read
   control still passes, and the five-thousand-poll session still posts.
3. The responder may accept two connections; do not redesign the test harness around a single-accept server.

## TDD Contract
- **RED first.** Add the test, then reintroduce cumulative out-accounting on the write path; the new test must
  fail with `net: byte budget exhausted` (the daemon's own string), and the count of tests must be four.
- **GREEN.** With the committed accounting (`self.sent`) the new test passes and the file shows
  `4 passed; 0 failed`.

## Exit Criteria
- [ ] `cargo test -p pii-net --test budget_survives_a_day`
- [ ] `cargo test -p pii-net --test budget_survives_a_day -- --exact a_write_window_is_spent_per_connection 2>&1 | grep -q 'test result: ok. 1 passed'`

## Guardrails
- Do NOT weaken or delete an existing test: the file must end with **four** tests, not three.
- Do NOT raise the window constant to make the test pass. The subject is the lifetime the window is applied
  over, not its size.
- Do NOT change the refusal string `net: byte budget exhausted`: the deployed daemon logs it and the incidents
  reader matches it.
- Do NOT touch `specs/`.
- If the write path cannot be made to observe the window in a test (for instance the responder cannot accept two
  simultaneous writers without deadlocking the harness), **STOP** and report the shape you would need instead of
  testing something else and calling it this criterion.

## Closure
(to be filled by the orchestrator: the two criteria run by hand, the mutation that reddens the new test, and the
`sha256sum` before/after of the file it mutated.)
