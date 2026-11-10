# Spec: 130 — the boss launcher dials the daemon like everyone else

**Project**: <repo>
**Phase**: after the `057` plan (111–129). Filed 2026-10-10 by the orchestrator while verifying specs 123–127: the
sweep found `pii-evolve`'s boss launcher dialling the daemon control socket **directly**, and a net-check
allowlist entry withholding the fact.

## Overview

`pii_uds` exists so that every client dials the daemon the same way: `client_connect_observed(sock, &observer)`
emits `names::UDS_CONNECT_FAILED` with the endpoint, so a dead daemon is *visible in the ledger* instead of being
a silent `Err`. `pii-cli` uses it (`crates/pii-cli/src/cmd_ctl.rs:29`).

`crates/pii-evolve/src/driver.rs:143` does not:

```rust
let stream = tokio::net::UnixStream::connect(&cfg.socket)
    .await
    .map_err(|e| PiiError::Unavailable(format!("connect {}: {e}", cfg.socket.display())))?;
```

and `xtask/src/net_check.rs:55-61` carries an allowlist entry for that file, commented as production code. So the
one component whose job is to *launch work onto the daemon* is the one component whose connect failures never
reach the observability seam — and a sanctioned exemption hides it.

This is the same class the register already carries twice: a workaround that survives because the check that
would catch it was told to look away.

## Requirements

### Functional

- **R1 — one dial path.** `pii-evolve`'s launch connects through `pii_uds::client_connect_observed` with an
  observer. No `tokio::net::UnixStream::connect` remains in `pii-evolve`'s production code.
- **R2 — the exemption goes.** The `crates/pii-evolve/src/driver.rs` entry is removed from
  `xtask/src/net_check.rs`'s allowlist (the test file's entry may stay if the test genuinely dials raw — say
  which and why).
- **R3 — a dead socket is observable.** A connect failure emits `names::UDS_CONNECT_FAILED` carrying the
  endpoint, through whatever observer the caller supplies (`NullObserver` when the caller has none — the event
  then exists in the seam, not in the ledger; say which one the launcher passes and why).
- **R4 — the launcher's behaviour is unchanged.** `run_cycle` still returns a `PiiError` whose message names the
  socket path and the OS error, and the existing `pii-evolve` tests stay green with their claims intact.
- **R5 — the check still bites.** Prove by mutation that net-check fails when a direct dial reappears in
  `pii-evolve`'s production code without an allowlist entry.

### Non-Functional

- 450-line file rule; `#![forbid(unsafe_code)]`; no new dependency; tests create the scratch they use under
  `_tmp` via the sanctioned helper; `cargo fmt` clean; `clippy -D warnings` clean.

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `launcher_dials_through_the_uds_client` | a daemon socket that answers `hello` | the launch succeeds; the failure is reported through `client_connect_observed`, not a raw `UnixStream::connect` |
| `launcher_reports_a_missing_socket_with_the_path` | `socket` pointing at a path that does not exist | `Err(PiiError::…)` naming that path; the observer records `UDS_CONNECT_FAILED` with `endpoint` = that path |
| `net_check_refuses_a_direct_dial_in_pii_evolve` | the allowlist entry removed and a direct dial put back | `net-check` exits non-zero and names the file |
| `no_raw_dial_remains_in_pii_evolve` | `crates/pii-evolve/src/*.rs` | no `UnixStream::connect` outside the UDS crate's own client |

## Exit Criteria

- [ ] `cargo test -p pii-evolve` passes (all suites), with the two new tests above present and passing
- [ ] `grep -rn 'UnixStream::connect' crates/pii-evolve/src/` returns nothing
- [ ] `cargo run -q -p xtask -- net-check` exits 0, and `grep -c 'pii-evolve/src/driver.rs' xtask/src/net_check.rs` is 0
- [ ] the mutation in R5 reddens `net-check` (the mutation and its output pasted in the report, then restored byte-identical)
- [ ] `bash quality-gate.sh` passes in the dev tree

## Guardrails

- Do NOT change `pii_uds`'s API to accommodate the call site; the call site adapts.
- Do NOT add an allowlist entry that keeps the direct dial.
- Do NOT "fix" this by making `net_check` ignore `pii-evolve` wholesale.
- Do NOT touch `specs/`.
- If the connect-failure event cannot carry the endpoint through the launcher's observer without changing
  `pii_uds`'s signature, STOP and report the conflict rather than widening the API.

## Orchestrator verification (2026-10-11)

**My own mutation (independent of the worker's R5):** a direct dial appended to `crates/pii-evolve/src/driver.rs`
(the worker had mutated `lib.rs`) → `net-check` exited 1 with
`NET-CHECK FAIL: crates/pii-evolve/src/driver.rs:176 — UnixStream::connect outside pii-net`; restored
byte-identical (`f72b733bf8a37e0d…`) → `net-check` rc 0. The exemption is gone and the check still bites.

**The production caller goes through a real observer** — this is what makes R3's intent true rather than
nominal: `crates/pii-evolve/src/main.rs:50` calls `run_cycle_observed(&cfg, &StderrObserver)`. Only the
convenience wrapper `run_cycle` (no observer available) passes `NullObserver`, as R3 allows.

**Cross-checked, not taken from the report:** `grep -rn 'UnixStream::connect' crates/pii-evolve/src/` → nothing
after the final edit (the worker's criterion 2 ran before that edit); the `driver.rs` allowlist entry is gone
from `xtask/src/net_check.rs` (only the test's entry, for a real stub daemon, remains); `cargo test -p pii-evolve`
= 5 + 2 + 2 green; `deps-check`/`doc-drift-check`/`lint-observability` rc 0 with 0 findings.

**Noted weakness (kept, not hidden):** `launcher_dials_through_the_uds_client` also asserts the *source text*
contains `client_connect_observed`. A text check can pass while another raw dial exists — which is why the claim
is really carried by `net-check` and `no_raw_dial_remains_in_pii_evolve` (both proven by mutation), not by the
string match.

## Closure (2026-10-11)

**Committed** `4a70f23`. **Gate:** plain `fresh-gate` on the tree carrying it — `quality-gate: ALL PASS · fresh-gate: PASS` (14 steps).

