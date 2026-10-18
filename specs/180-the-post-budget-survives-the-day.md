# Spec: the post budget survives the day

## Overview
MEASURED 2026-10-17 22:39Z, verbatim from the deployed daemon log:

    {"error":"capability denied: net: byte budget exhausted","channel":"discord"} kind=channel.egress.denied

027895e moved the net byte window to the CONNECTION, and the post is still denied under a daemon that has made hundreds of requests. Whatever accumulates must be bounded per poll or per run rather than over the daemon life, and the refusal must stay loud.

**Project**: /projects/pi-improved

## Requirements
A channel or a cron run must be able to run for a day without exhausting its byte budget, with a scripted test that makes thousands of requests and still posts, AND a single oversized response still refused.

## Exit Criteria
- [ ] cargo test -p pii-net --test budget_survives_a_day
- [ ] cargo test -p pii-channels --test discord

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
- Do NOT simply raise the constant: the defect is the lifetime the window is applied over.

## Closure (2026-10-18, orchestrator, measured here and not taken from the worker's report)

**The two criteria, run by hand on the committed tree (`6055361`):**

```
cargo test -p pii-net --test budget_survives_a_day      -> test result: ok. 3 passed; 0 failed   (rc=0)
cargo test -p pii-channels --test discord              -> test result: ok. 2 passed; 0 failed   (rc=0)
```

**The test count did not move down.** `1e5d877` changes only `crates/pii-net/src/service/conn.rs`
(21 insertions, 15 deletions) and no test file, so a criterion that asserts `test result: ok` was not
satisfied by deleting tests.

**My own mutation, which reddens the fix.** The defect was cumulative accounting over the grant key;
restoring it on the **read** path (`*total` from `registry.bytes_in` compared to `max_bytes_in` instead of the
connection's own `received`) gives:

```
cargo test -p pii-net --test budget_survives_a_day  -> FAILED, 2 of 3 red
  thousands_of_requests_then_a_post_still_succeeds: request 487 refused: capability denied: net: byte budget exhausted
  a_concurrent_connection_cannot_spend_this_ones_window
```

`conn.rs` was restored byte-identical: `sha256 48fde844c4aee8d0f215cfe9b73d1bb36594a97a989bb6f06b42f8f90e1f0c13`
before and after, and `git status --porcelain -- crates/pii-net/` is empty.

**A false green, recorded rather than folded into this closure.** The symmetric mutation on the **write** path
(the same cumulative comparison over `bytes_out`) stayed **green: 3 passed**. No test in this file moves more
than about a kilobyte through one connection's `write_all` (the polling test's traffic is charged to
`max_bytes_in`, not `max_bytes_out`), so a re-shared out-window cannot be observed by this suite. The read path
is proven; the write path's per-connection window is not. Carried as row A48 / spec 181 instead of being
called covered here.
