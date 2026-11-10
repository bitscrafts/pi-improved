# Spec 325: a health answer that is not this build says so

## Overview

**Project**: <repo>
**Phase**: production identity / the deploy gate rank 1 of 059 §4 names

On 2026-10-31 the running daemon was asked who it is, with a client built at `HEAD ae10568c0456`:

```
$ pii health
pii 0.1.0 (0a78d6da0c20, x86_64-unknown-linux-gnu)
...
since_deploy:
  revision: 0a78d6da0c20
  runs: 13
  posted: 0
```

It answered from **`0a78d6da0c20`** — an 11-day-old build (the installed binary is dated 2026-10-20, and its
usage line has no `lane` subcommand, so it predates specs 317/319/320) — and `pii health` **exited 0**. Every
probe that reads `rc 0` as "healthy" therefore certified a build this tree no longer contains. That reading is
written down twice: `deploy/pii.service` line 11 and `_docs/05-operations/105-runbook.md` line 20 both say
*"probe: non-zero exit = not running"*, which is exactly the reading the stale binary satisfies while being
wrong about the only question an operator cares about — whether production is the tree.

The pieces already exist and nothing joins them: `sys.status` carries `/build/revision`
(`BuildIdentity`, spec 99), the client carries its own revision (`pii_server::daemon::identity::REVISION`,
printed by `pii lane rev` as of spec 320), and `pii health` prints the daemon's. No command compares the two, so
"the daemon is healthy" and "the daemon is *this* daemon" are the same output. This spec separates them: a
health answer that is not this build says so, with both revisions named and a code of its own.

## Requirements

**R1 — three states, three codes.** `pii health` returns `0` when the daemon's `/build/revision` equals this
client's `identity::REVISION`; `2` when no daemon answers (today's behaviour, unchanged); `3` when the daemon
answers but is not this build, or when the comparison cannot be made because either revision is absent, empty, or
the literal `unknown`. No other meaning is added to `0`, `2` or `3`.

**R2 — drift is named, on stderr, with both revisions.** When the two known revisions differ, exactly one line
goes to stderr:

```
DRIFT: the daemon answers from <daemon-rev>; this client is <client-rev> — the running binary is not this tree
```

stdout keeps the whole payload (the human render, or `--json`), so the reader still sees the identity that was
answered — the drift line adds a fact, it does not replace one.

**R3 — unknown is not agreement.** If either side's revision is missing, empty, or `unknown`, the exit is `3`
and the line is:

```
UNVERIFIED: <daemon|client> build revision is unknown — this client cannot prove the daemon is this build
```

**R4 — the human output names the client's build too.** `render` gains one line immediately after its first
line: `client: <rev>`, or `client: unknown`. One command then shows both revisions. `--json` is unchanged: it
remains the daemon's payload, nothing added and nothing removed (tests assert on the parsed payload, not on
bytes, because the client re-serialises the value it parsed).

**R5 — the runbook and the unit say what each code means.** The probe line in
`_docs/05-operations/105-runbook.md` and the probe comment in `deploy/pii.service` each carry the sentence
`0 = running and this build, 2 = no daemon, 3 = a daemon that is not this build`, replacing today's
"non-zero = not running" — false the moment this lands.

**R6 — the comparison is a pure function, tested without a daemon.** `fn verdict(daemon: &str, client: &str)`
returning `Same | Drift | Unverified(<side>)`, unit-tested for same, different, unknown on either side, and
missing on either side. The daemon-serving cases are integration tests.

**R6 — an export has no revision, so `3` is the right answer there (amended 2026-10-31).** A fresh-gate export
is `git archive` of HEAD: no `.git`, so `identity::REVISION` is `unknown` on both sides and R3 makes every
`pii health` in that tree exit `3`. Consequence for anything asserting success: a test whose subject is NOT the
verdict must accept the three documented codes (`0`, `2`, `3`) rather than `success()`. Measured: the first
three attempts at this spec parked on
`control_plane_is_not_behind_runs::a_kill_cancel_does_not_freeze_the_async_workers`
(`assert!(health.status.success())`, line 285) — the code was right, the assertion predated R1.

**R7 — the stub daemon is a sanctioned test-fake.** `health_drift.rs` serves its own `control.sock` answering
`sys.hello`, because a real daemon on this build can never answer with a foreign revision. `net-check` refuses
any `UnixListener::bind` outside `pii-net`, so the file joins that checker's allowlist with its reason —
the same class as the entries for specs 118/200/230/241/252/263.

## Files

- `crates/pii-cli/src/cmd_health.rs`
- `crates/pii-cli/tests/health_drift.rs`
- `crates/pii-cli/tests/control_plane_is_not_behind_runs.rs` (R6: the verdict set, timing subject untouched)
- `xtask/src/net_check.rs` (R7: the allowlist entry and its reason)
- `_docs/05-operations/105-runbook.md`
- `deploy/pii.service`

## TDD Contract

The stub daemon's payload is **taken from the live daemon in the same test** (run `--json` against a daemon the
test started, keep the payload, change exactly one field), so a hand-written fixture cannot drift from the wire
shape the daemon really produces.

| Test Name | Input | Expected Output |
|---|---|---|
| health_verdict_same | identical revisions | `Same` |
| health_verdict_drift | two different known revisions | `Drift` |
| health_verdict_unknown_daemon | `"unknown"`, a known rev | `Unverified("daemon")` |
| health_verdict_unknown_client | a known rev, `"unknown"` | `Unverified("client")` |
| health_verdict_missing | `""`, a known rev | `Unverified("daemon")` |
| health_agrees_with_a_daemon_of_this_build | a real daemon from `CARGO_BIN_EXE_pii`, `PII_RUNTIME_DIR` | rc 0, stdout carries `client: <REVISION>`, no `DRIFT` |
| health_names_a_drift | stub serving the live payload with `/build/revision` = `0123456789ab` | rc 3, stderr carries `DRIFT:` and both revisions, stdout still the payload |
| health_refuses_unknown | stub serving the live payload with `/build/revision` = `unknown` | rc 3, stderr carries `UNVERIFIED:` and names the daemon |
| json_is_the_daemons_payload | `--json` against the stub | parsed payload equals the served one; rc 3 |
| no_daemon_is_still_two | no daemon on the runtime dir | rc 2 and the existing message, unchanged |

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test health_drift 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-cli --bins health_verdict 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `cargo test -q -p pii-cli --test control_plane_is_not_behind_runs 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo run -q -p xtask -- net-check 2>&1 | tail -1 | grep -q "PASS: net-check"`
- [ ] `grep -q "0 = running and this build, 2 = no daemon, 3 = a daemon that is not this build" deploy/pii.service && grep -q "0 = running and this build, 2 = no daemon, 3 = a daemon that is not this build" _docs/05-operations/105-runbook.md`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Guardrails

- Do NOT compare versions: every build since the first release says `0.1.0`. Revisions only.
- Do NOT add a field to `sys.status` or reword the payload: the wire contract is spec 99's, and the comparison
  is client-side because only the client knows what *it* was built from.
- Do NOT add a daemon-side override for the reported revision (no `PII_REPORTED_REV`): the drift case is served
  by a stub whose payload is the live one with exactly one field changed.
- Do NOT fold this state into `2`. "No daemon" and "a daemon that is not this build" are different faults, and
  the runbook's probe depends on telling them apart.
- Do NOT weaken the existing unit tests or the existing "no daemon answered" message to make room.
- The five existing `lane-*` tests clone `<repo>` by absolute path; this spec touches none of
  them. If a test must reach the dev tree, declare `# requires: dev-tree` (spec 324 R6) rather than guessing a
  path.
- STOP if `pii_server::daemon::identity::REVISION` is not reachable from `pii-cli` (it is today, through the
  `pii-server` dependency, and `pii lane rev` already prints it) — say so instead of inventing a second
  identity source.

## Closure

(outcome, the RED before, the live `pii health` answer against the running daemon after — with its new exit code
— and the two documentation lines that changed: filled in when the item is closed.)
