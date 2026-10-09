# Spec: 88 — lag recovery must not re-request approvals, and must recover lost `RunFinished` (N3)

**Project**: /projects/pi-improved
**Phase**: 045 re-review, candidate 4 — completing D6 (spec 79 closed only the timer half).

## Context (read by the reviewer, orchestration confirms the call chain)
`crates/pii-server/src/daemon/lag.rs::recover_suspended` calls `on_suspended` for every
`Suspended` run. Its doc claims "Idempotent — the wheel is keyed by (fire time, run)".
That is true for TIMER suspensions (`daemon/mod.rs:343-346` parses the fire time from the key).
It is FALSE for approvals: `on_suspended`'s approval arm (`mod.rs:348-350`) calls
`approvals::request` (`daemon/approvals.rs:48-75`), which **unconditionally** `put_approval`s a row
with `decision: None` and a fresh `requested_at_ms`, then re-broadcasts `ApprovalRequested`. So on
every lag: duplicate human notifications, and — in the window between `approve.resolve` recording a
decision and the run leaving `Suspended` — the recorded decision is ERASED.

Separately: lost `RunFinished` events are not recovered at all. `on_finished`
(`mod.rs:358-385`) re-arms cron and sends channel replies; the lag rescan never touches it. 043's D6
named both event kinds.

## Requirements
R1. Approval recovery is idempotent: `recover_suspended` (or the approval path it calls) must NOT
    create a second row or re-broadcast `ApprovalRequested` when a pending approval for that
    (run, key) already exists, and must NEVER overwrite a row that already carries a decision.
    Check-then-act inside the same lock/store transaction as the insert.
R2. On `Lagged`, also recover lost `RunFinished`: re-drive what `on_finished` would have done for
    runs whose terminal event may have been dropped — at minimum cron re-arm for cron-owned runs and
    the channel reply path for channel-owned runs (follow whatever bookkeeping already exists; do not
    invent new state).
R3. TDD, RED first, both must be observed failing before the fix:
    a. decide an approval, keep the run `Suspended`, call `recover_suspended` → the decision must be
       unchanged AND zero new `ApprovalRequested` events must have been broadcast (assert by counting
       received events on a subscriber, not by reading code);
    b. drop a cron run's `RunFinished` under a lag → the next firing still starts.
R4. State the honest limit in the report: which `RunFinished` consumers the recovery does NOT cover.

## Exit Criteria
- [ ] `cargo test -p pii-server 2>&1 | grep -c 'test result: FAILED' | grep -q '^0$'`
- [ ] the two named tests exist and pass; the report shows their RED output before the fix
- [ ] `cargo test -p pii-server lagged 2>&1 | grep -c 'test result: ok' | grep -qv '^0$'` (spec 79's test still passes — no regression)
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the four files touched and the exact call chain (`lag.rs` → `on_suspended` →
      `approvals::request` / `on_finished`) are listed in the report

## Guardrails
- Do NOT change the approval RPC surface, the wire event shape, or the store schema.
- Do NOT make recovery fire on the normal path — only on the lagged arm (an exceptional path).
- Do NOT re-request an approval that a human already answered. That is the defect being fixed.
- Do NOT commit; leave the tree for the orchestrator. Scratch under `_tmp/` only.