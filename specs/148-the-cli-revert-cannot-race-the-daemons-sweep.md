# Spec: the CLI's learn revert cannot race the daemon's sweep

## Overview

Filed with spec 125 and still open in `_docs/05-operations/open-items.md` §9: `pii learn revert` from a CLI process
and the daemon's own auto-revert sweep can act on the same promotion at the same time — two writers on one
record. This is the same class as the audit fork (§1), whose resolution the project already chose and shipped in
spec 138: **one writer**, the daemon applies and records.

**Project**: /projects/pi-improved
**Phase**: the gate's own robustness (pre-video backlog, specs 145–148)

## Requirements

### Functional

R1. When a daemon owns the store, a CLI revert is a **request**, applied by the daemon through the single-writer
    path spec 138 built. The CLI does not write the promotion record itself in that case.
R2. When no daemon owns the store, the CLI may apply the revert directly — the documented degraded path — and it
    must say which path it took in its output.
R3. Two reverts of one promotion (one from the CLI, one from the sweep) result in exactly one application and one
    "already reverted": idempotent, with no double bookkeeping in the ledger.
R4. The decision ledger records which principal acted (`cli` or `daemon`) for every revert.

### Non-Functional

R5. The request path must not lose a revert when the daemon is busy: the request is durable (the store it lands
    in survives a restart) and the daemon applies it on its next pass.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| a_cli_revert_is_a_request_when_the_daemon_owns_the_store | daemon holding the store, CLI revert | the daemon applies it once; the CLI writes no record |
| a_cli_revert_applies_directly_without_a_daemon | no daemon | applied, and the output says which path |
| two_reverts_are_one_revert | CLI + sweep on the same promotion | one application, one "already reverted" |
| the_ledger_names_the_principal | the same | the ledger row says cli or daemon |

## Exit Criteria

- [ ] `cargo test -p pii-learn -p pii-server --lib` prints `test result: ok`
- [ ] `grep -c 'append(true)' crates/pii-learn/src/**/*.rs` is 0, or every site is reachable only from the daemon
      (the same criterion spec 138 set for promotions)
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT add a second writer. Reuse spec 138's path.
- Do NOT silently ignore a revert request when the daemon is not running; print the path taken.
- Do NOT make the CLI wait indefinitely for the daemon.
