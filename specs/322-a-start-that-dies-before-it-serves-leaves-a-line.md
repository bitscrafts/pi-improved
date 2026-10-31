# Spec 322: a start that dies before it serves leaves a line, and the scan says so

## Overview

`pii.log` records `daemon.started` — but only *after* the store is open, the engine built and the
control socket bound (`cmd_daemon.rs:123` opens the log, `:403` writes `daemon.started`). A start that
dies before that point leaves **nothing**: the supervisor knows, the log does not. Measured on the
deployed log (2026-10-31, copy of `/root/.local/state/pii/pii.log`): **108 `daemon.started`, 0 `daemon.ready`**,
and the runtime scan's own verdict was

```
starts_considered=108  incidents=1937
  run-stalled                1851
  daemon-restart-unexpected    74
  refusal-repeated             12
  daemon-start-not-ready        0
```

That zero is **vacuous**: the scan looks for a start mark (`START_MARK = daemon.sandbox.in_force`,
`incident_runtime.rs:21`) that production never writes — the name is present 0 times in 5.6 MB of
deployed log. So `daemon-start-not-ready` cannot fire, and 063 §Phase B ("`daemon.started` and the start
mark are written to the same file") is unmet.

This spec makes a start observable from its first moment and makes the incident decidable — without
turning a scan that races a healthy boot into a false alarm.

## Requirements

### R1 — the daemon writes `daemon.starting` before anything that can fail
The mark is emitted immediately after the log is opened, before the store is opened, the engine is
built, or the socket is prepared. Worked example: `pii daemon --store /nonexistent-dir/store.redb`
produces a log whose **first** line is `daemon.starting` with the pid, and **no** `daemon.started`.

### R2 — `daemon.started` keeps its meaning
It is still written after the socket is bound and accepting (`cmd_daemon.rs:403`), with the same
fields. A healthy boot therefore shows `daemon.starting` → `daemon.started`.

### R3 — the scan pairs mark and start, and fires only when the pair is broken
Input (JSONL, one event per line; `t` = seconds):
- `daemon.starting` @0, `daemon.started` @2 → **0** incidents (control: a normal start).
- `daemon.starting` @0, `daemon.starting` @2 → **1** `daemon-start-not-ready` naming the first mark.
- `daemon.starting` @0, log ends at @1 → **0** incidents (control: a scan racing a boot).
- `daemon.starting` @0, log ends at @600 → **1** `daemon-start-not-ready` (the start never served).
- legacy `daemon.sandbox.in_force` @0 + @30 → still **1** (old logs keep their meaning).

### R4 — the incident names both ends
The incident's evidence carries the mark timestamp and the timestamp at which the start was judged
lost (the superseding mark, or the log end), so an operator can find the line.

### R5 — the deployed record does not acquire false positives
`pii incidents scan --source runtime --log <copy of the deployed pii.log>` still reports
`daemon-start-not-ready` **0** times on the pre-change log.

## Files

- `crates/pii-observe/src/names.rs`
- `crates/pii-cli/src/cmd_daemon.rs`
- `crates/pii-learn/src/incident_runtime.rs`
- `crates/pii-learn/tests/incidents_runtime_start_mark.rs`
- `crates/pii-cli/tests/daemon_start_mark.rs`

## TDD contract

| Test | Input | Expected output |
| --- | --- | --- |
| `a_start_that_never_served_is_one_incident` | `starting@0, starting@2` | 1 incident, fingerprint `daemon-start-not-ready` |
| `a_healthy_start_is_silent` | `starting@0, started@2` | 0 incidents |
| `a_scan_racing_a_boot_does_not_cry` | `starting@0`, log end @1 | 0 incidents |
| `a_start_lost_for_ten_minutes_is_reported` | `starting@0`, log end @600 | 1 incident |
| `the_legacy_sandbox_mark_still_pairs` | `sandbox.in_force@0, sandbox.in_force@30` | 1 incident |
| `the_daemon_logs_starting_before_it_can_fail` | `pii daemon --store /proc/nonexistent/store.redb` | first log line `daemon.starting`, no `daemon.started`, rc≠0 |
| `the_deployed_log_gains_no_false_positive` | copy of `pii.log` | 0 `daemon-start-not-ready` |

## Exit Criteria

- [ ] `cargo test -q -p pii-learn --test incidents_runtime_start_mark`
- [ ] `cargo test -q -p pii-learn --test incidents_runtime`
- [ ] `cargo test -q -p pii-cli --test daemon_start_mark`
- [ ] `cargo clippy -q -p pii-learn -p pii-cli -p pii-observe --all-targets -- -D warnings`
- [ ] `grep -q 'DAEMON_STARTING' crates/pii-observe/src/names.rs`
- [ ] `test "$(grep -n DAEMON_STARTING crates/pii-cli/src/cmd_daemon.rs | head -1 | cut -d: -f1)" -lt "$(grep -n 'Engine::new' crates/pii-cli/src/cmd_daemon.rs | head -1 | cut -d: -f1)"`
- [ ] `test "$(target/debug/pii incidents scan --source runtime --log _tmp/night/pii.log.copy --root _tmp/night/root | grep -c daemon-start-not-ready)" = 0`

## Guardrails

- Do NOT weaken the existing `daemon-restart-unexpected` rule, and do NOT let a healthy boot become an
  incident: the log-end grace is what separates "died" from "still booting", and it must be tested in
  both directions.
- Do NOT delete the legacy sandbox-marker rule; old logs must keep their meaning.
- The daemon must not gain a new failure mode: a mark that cannot be written is a warning, never a
  reason to refuse to start.
