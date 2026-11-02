# Spec: `pii health` answers from a plain shell, because the client and the unit agree on the socket

## Overview

```
$ ssh mvcorrea@lnx '~/.local/bin/pii health'
error: no daemon answered (… /run/user/1000/pii/default/control.sock: No such file or directory)
```

With `PII_RUNTIME_DIR=/run/user/1000/pii` exported it answers `pii 0.1.0 (57129b83a0d4)`. The unit sets that
variable (`Environment=PII_RUNTIME_DIR=/run/user/1000/pii`); a login shell does not. So the operator's plain
command — the one every runbook and every support conversation starts with — fails.

The `default` suffix is the tell: when the variable is unset the **client** appends a channel/user segment that the
**daemon**, started with an explicit directory, never created. Two different ideas of "the default socket" in one
binary.

**Project**: /projects/pi-improved
**Phase**: production hardening (Opus audit, M5's operator path)

## Requirements

**R1 — one default, computed once.** The control socket path is computed by one function; the daemon and the
client call it. When `PII_RUNTIME_DIR` is set, both use it verbatim. When it is unset, both derive the same path
from the uid (`/run/user/<uid>/pii`) — including whatever sub-segment the design needs, so that the unit's explicit
value and the derived value name **the same file**.

**R2 — the unit and the derived default agree, and a test says so.** A test compares the path the client resolves
with no environment against the path in the shipped unit file; if they differ it fails, naming both.

**R3 — the failure is actionable.** When the socket is absent, the error names the path it tried **and** the
service that would create it (`systemctl --user status pii`), instead of `is pii daemon running?`.

**R4 — no behaviour change for a caller that sets the variable.** Every existing script that exports
`PII_RUNTIME_DIR` keeps working; this spec removes a difference, it does not add a mode.

## TDD Contract

| Test | Input | Expected |
|---|---|---|
| `the client and the daemon resolve the same socket with no environment` | `env -u PII_RUNTIME_DIR` | equal paths |
| `an explicit PII_RUNTIME_DIR is used verbatim` | `PII_RUNTIME_DIR=/tmp/x` | both are `/tmp/x/control.sock` |
| `the shipped unit's value equals the derived default` | the unit file on disk and the derived path | equal |
| `control: the old resolution differs` | the binary from `git show HEAD` | the two paths differ (the defect) |
| `a missing socket names the service` | no daemon | the message contains `systemctl --user status pii` |

## Exit Criteria

- [ ] `cargo test -q -p pii-uds --test one_default_socket 2>&1 | grep -qE "test result: ok\. [5-9] passed"`
- [ ] `cargo test -q -p pii-cli --test health_without_env 2>&1 | grep -qE "test result: ok\. [1-9] passed"`
- [ ] `! grep -rn 'join("default")' crates/pii-cli/src crates/pii-uds/src`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` → `fresh-gate: PASS`

## Guardrails

- Do NOT remove `PII_RUNTIME_DIR` support: systemd and containers both need it.
- Do NOT make the client scan directories for a socket.
- Do NOT change the socket's permissions or the daemon's ownership model.
- The production unit is not edited here; the test only proves the shipped file agrees with the code.
