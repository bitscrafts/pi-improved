# Spec: one writer for promotions

## Overview

The promotion path has two writers and no lock. Measured 2026-10-11
(`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 Q6, F5): `learn/` is written both by the CLI
(`pii learn promote` / `revert`, `crates/pii-learn/src/promote/mod.rs`) and by the daemon's auto-revert sweep,
and `append_ledger`'s sequence number is allocated by **reading and counting the file before appending**
(`crates/pii-learn/src/promote/storage.rs:75-93`). Two writers that overlap can therefore produce duplicate
`seq`, or lose a save entirely — measured by the reviewer's 50-round revert-versus-sweep harness. The
identical problem in the same project was already solved once, in the entity store, with a lockfile
(`crates/pii-memory/src/entity/*`): the pattern exists here, it was just not used on this path.

This is also the audit-trail fork the register has been carrying: the loop's record lives in three places
(`approval.redb` for human approvals, `learn/ledger.jsonl` for applied/reverted promotions, the decision ledger
for refusals), and the promotion's own record is the one a person cannot trust while two writers race.

**Decision (orchestrator, 2026-10-11, autonomy): option B.** The promotion becomes an RPC that the daemon
applies **and** records, in one step. The daemon already owns the store and is already the only writer of the
decision ledger; the CLI keeps its command surface but stops writing the ledger directly.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (finding 059 F5, Q6; `RESUME-REGISTER` audit-trail fork)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 (F5, Q6), §2.3

## Requirements

### Functional
- **R1** `learn/ledger.jsonl` (or its successor) has exactly **one writer**: the daemon. Any path in
  `pii-learn` that appends to it from a library caller is either removed or reachable only from the daemon.
  The CLI asks the daemon; if the daemon is not running, the command fails with a clear "the daemon owns the
  ledger" message rather than writing directly (fail-closed, never a silent local write).
- **R2** Apply-and-record are **one step**: a promotion that is applied is recorded, and a promotion that is
  recorded was applied. A crash between the two must not produce a record without an effect or an effect
  without a record — say in the report which of the two it prefers and why.
- **R3** The sequence number is allocated **under the same writer's lock** as the append (no
  read-then-append). If the daemon cannot be the only writer for a genuine reason, a lockfile following the
  entity store's pattern is the fallback — but the fallback must be stated, with its cost, in the report.
- **R4** The refusal path (spec 131) keeps its keying: a refusal is a decision keyed by the promotion id in
  the decision ledger, and the promotion's own record must be joinable to it by the same id. State the join
  explicitly in the report — that is the deliverable the register asked for.
- **R5** Tests: (a) the reviewer's race is reproduced and must come back clean: a 50-round harness running
  revert-versus-sweep concurrently shows **0 duplicate seq and 0 lost records**; (b) a CLI call with no daemon
  running fails closed with the named reason; (c) a restart of the daemon after a promotion is applied still
  reads the record (the existing persistence tests must keep passing).

### Non-Functional
- No new dependency. The daemon's RPC surface reuses the existing UDS request/response types (spec 130's
  `pii_uds`), and a new request kind must go through the same observed client (no raw dial — `net-check` is
  the enforcement).
- The JSONL (or its successor) stays readable by a human and by the existing tests: do not replace it with a
  binary store in this spec.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `concurrent_revert_and_sweep_do_not_duplicate_or_lose` (new) | 50 rounds, the reviewer's harness | no duplicate `seq`, every applied promotion present exactly once |
| `the_cli_refuses_to_write_the_ledger_without_the_daemon` (new) | CLI promote with no daemon | fails closed, message names the daemon as the writer |
| `a_promotion_reads_back_after_a_restart` (new/existing) | apply, restart the daemon, read | the record is present and joinable by promotion id |
| the 131 refusal tests | unchanged | still green, keyed by the promotion id |

## Exit Criteria

- [ ] `cargo test -p pii-learn 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-store 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo run -q -p xtask -- net-check 2>&1 | tail -1 | grep -qv 'FAIL'`
- [ ] `grep -rn 'OpenOptions::new().append(true)' crates/pii-learn/src | grep -v 'daemon' | wc -l` is 0, or the report names each remaining site and why it is not a second writer

## Guardrails

- Do NOT "fix" the race by making the two writers wait for each other with a sleep or a retry loop.
- Do NOT delete the existing ledger or migrate data destructively; anything already written must stay
  readable. Never delete a file you did not create (HARD RULE 0).
- Do NOT change the promotion *semantics* (what makes a candidate eligible, the TTL, the auto-revert rule) —
  this spec is about **who writes**, not about what is decided.
- If the daemon cannot own the ledger without a change larger than this spec (a new node kind, a new
  subsystem), **STOP and report** the shape you found instead of forcing it.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `/projects/_tmp/`.

## Closure

(outcome, the 50-round harness result before and after, the single writer's identity in the tree, and the join
key stated — filled in when the item is closed.)
