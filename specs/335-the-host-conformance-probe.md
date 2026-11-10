# Spec: The host conformance probe — the five things that are true of the container and false of production

## Overview

Opus audited this project on 2026-11-02 (`_docs/06-research/opus-status-and-path-20261102.md`) and found five
defects on the host that **no gate sees**, because all 15 gate rows and all 42 selftest rows run in the `hermes`
container against the container's config. Production is a different machine, user, binary path, config file and
installer. Measured on the host that day:

```
$ ssh <operator>@<host> '~/.local/bin/pii health'
error: no daemon answered (… /run/user/1000/pii/default/control.sock: No such file or directory)

$ ssh <operator>@<host> 'grep -nE "/root|/projects" ~/.local/pii/pii.toml | grep -v "^[0-9]*:#"'
53:trajectory_dir = "~/.local/state/pii/trajectory"
63:log_file = "~/.local/state/pii/pii.log"

$ ssh <operator>@<host> 'findmnt -no FSTYPE /run/user/1000'
tmpfs                      # channel.redb and timer.redb live there (daemon/mod.rs:201,204)

$ ls _tmp/autonomy/INCIDENTS.md        # absent, and its publisher exits 0 on absence
```

This spec builds the probe that turns those sentences into a command with an exit code.

**Project**: <repo>
**Phase**: production hardening (Opus audit, `f₁`)

## Requirements

**R1 — one command, five checks, one verdict.** `scripts/checks/host-conformance.sh` prints one `PASS`/`FAIL` line
per check, each naming the offending path or line, then a final `HOST-CONFORMANCE: <n> passed, <m> failed` and
exits non-zero when any check fails. The checks:

- (a) **config paths** — no `/root` or `/projects` path in the host config outside a comment;
- (b) **durable stores** — no store path under `/run/` (both `channel:` and `timer:` in `pii health`);
- (c) **health from a plain shell** — `pii health` succeeds with **no** `PII_RUNTIME_DIR` in the environment;
- (d) **no failures since the last start** — no `level":"error"` event of kind `*.failed` after the last
  `daemon.started` in the live log;
- (e) **no false start alarm** — `pii incidents scan --source runtime` reports `daemon-start-not-ready` for the
  current start zero times.

**R2 — it is a gate row.** `scripts/checks/gates.sh` gains `run M5 "host conformance: five checks"`. Because the
row needs a host, it must **skip loudly when `PII_HOST` is unset** — printing `SKIP (no PII_HOST)` and not
counting as a pass — never silently green.

**R3 — a fixture mode, so the probe itself is testable without a host.** `--fixture <dir>` reads a directory laid
out like a home (`pii.toml`, `state/pii/health.json`, `state/pii/pii.log`, `logs/incidents.txt`) and runs the same
five checks against it. This is what the test drives; the ssh path and the fixture path must share one
implementation of the five checks, not two.

**R4 — refuse, do not pass, when the host is unreachable.** `ssh` failing exits **2** with
`REFUSED: cannot reach <host>`; it must never be reported as five passes. (Same failure this project keeps
paying for: a check that cannot run is not a check that passed.)

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `five checks, all red, on today's host shape` | fixture reproducing the four measured defects | 5 FAIL, rc=1, each line naming its path |
| `a conformant fixture is green` | fixture with `/home/u/...` paths, stores under `state/`, plain health ok, no failures, 0 start alarms | 5 PASS, rc=0 |
| `control: one defect at a time turns exactly one check red` | five fixtures, each with one defect | exactly 1 FAIL each time, and it is the right one |
| `unreachable host refuses` | `PII_HOST=10.255.255.1` with a 2 s ssh timeout | `REFUSED`, rc=2 |
| `the gate row skips loudly without a host` | `env -u PII_HOST bash scripts/checks/gates.sh --gate M5` | prints `SKIP (no PII_HOST)`, not PASS |

## Exit Criteria

- [ ] `test -x scripts/checks/host-conformance.sh`
- [ ] `! bash scripts/checks/host-conformance.sh --fixture scripts/checks/fixtures/host-red.json >/dev/null 2>&1`
- [ ] `bash scripts/checks/host-conformance.sh --fixture scripts/checks/fixtures/host-red.json 2>&1 | grep -q 'HOST-CONFORMANCE: 0 passed, 5 failed'`
- [ ] `bash scripts/checks/host-conformance.sh --fixture scripts/checks/fixtures/host-green.json 2>&1 | grep -q '5 passed, 0 failed'`
- [ ] `bash scripts/checks/host-conformance.sh --fixture scripts/checks/fixtures/host-one-defect.json 2>&1 | grep -q '4 passed, 1 failed'`
- [ ] `bash scripts/checks/tests/host-conformance.test.sh 2>&1 | tail -1 | grep -qE 'RESULT: [1-9][0-9] ok, 0 FAIL'`
- [ ] `grep -q 'host-conformance.sh' scripts/checks/gates.sh`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q 'fresh-gate: PASS'`

## Guardrails

- **Read-only over ssh.** No `systemctl restart`, no write, no `pii` subcommand that mutates. The probe may run
  `cat`, `grep`, `findmnt`, `pii health`, `pii incidents scan`, `tail`.
- Do NOT make the probe pass by editing the host in this spec: it reports, it does not repair.
- Do NOT add a sixth check that needs a credential.
- The fixture mode must not become the only mode that works: the ssh path is the product, the fixture is its test.

## Files

- `specs/335-the-host-conformance-probe.md`
- `scripts/checks/host-conformance.sh`
- `scripts/checks/host-conformance.py`
- `scripts/checks/tests/host-conformance.test.sh`
- `scripts/checks/fixtures/host-red.json`
- `scripts/checks/fixtures/host-green.json`
- `scripts/checks/fixtures/host-one-defect.json`
- `scripts/checks/gates.sh`
