# Spec: one intake path, and it works without the daemon
#: depends=- status=pending block=-

## Overview

Measured 2026-11-21, three facts in a row that together describe a fork nobody chose:

1. `pii queue-spec.sh 435 3 3` refused with `REFUSE: unexpected dirty paths` because of one document belonging to
   another session. Four rows (435–438) could not enter the queue.
2. The refusal comes from a helper that **is not in the repository**: it lives at `/projects/_tmp/pii-queue-spec.sh`, and
   `ls scripts/autonomy/queue-spec.sh` says no such file. Unversioned, unreviewed, not in the gate.
3. The binary **already has the command**: `crates/pii-cli/src/cmd_queue.rs:17-22` routes `add`, `ls` and `claim`, and
   `pii queue` prints
   `usage: pii queue add --id <id> --spec <path> --title <text> --criteria <cmd> ... [--touches <path>]...`.

So there are two intake paths. The one in the repository needs the daemon (`pii queue ls` →
`uds.connect.failed /run/user/0/pii/control.sock`); the one outside the repository bypasses it by writing
`scripts/autonomy/plan.tsv` directly, and carries a hard-coded allowlist of tolerated dirty paths
(`pii-queue-spec.sh:10`) that refuses everything it does not recognise.

That is why the fork exists: **the versioned path requires a daemon that cannot run while a gate runs, and this
project gates constantly.** The bypass was the workaround, and the workaround is the thing with the allowlist.

`pii queue claim` also already exists (line 22), which is the command spec 433 assumed had to be invented. Any row
written against an imagined interface must be re-read against the one that exists.

## Requirements

- **R1** One intake path: `pii queue add`. The workspace helper is deleted once the binary's path can do what the helper
  did, and the deletion is the proof the row landed (the progress metric is scripts removed, not rows added).
- **R2** `pii queue add` works with **no daemon running**: the queue's store is local, and requiring a control socket to
  append a row to it is the indirection that created the bypass. The daemon may be informed when it is up; it must not be
  required.
- **R3** Intake classifies the dirty paths it finds and says which: **owned** (a row declares it), **foreign** (outside
  any row), **blocking** (`Cargo.toml`, `gate-steps.txt`, `scripts/autonomy/*`). A foreign path warns and does not
  block; a blocking path refuses, naming it.
- **R4** No hard-coded list of tolerant filenames anywhere: the classification is computed from `git status` and the
  rows.
- **R5** The same classification answers dispatch (spec 433), so intake and dispatch do not disagree about whose tree it
  is — the disagreement that made a hand-implemented row unlandable.

## TDD contract

| Test | Input | Expected |
| --- | --- | --- |
| intake without a daemon | no socket at `/run/user/0/pii/control.sock` | `pii queue add …` exits 0 and the row is listed by `pii queue ls` |
| a foreign path warns, does not block | tree dirty in `_docs/06-research/085-...md`, no row claims it | row added; warning names the path |
| a blocking path refuses | `gate-steps.txt` modified | exit non-zero, message names `gate-steps.txt` |
| the bypass is gone | `/projects/_tmp/pii-queue-spec.sh` | absent |

The first row is the property the fork existed to work around; the third is the control that a tool which stopped
refusing would fail.

## Exit Criteria

- [ ] `test ! -e /projects/_tmp/pii-queue-spec.sh`
- [ ] `pii queue add --id 9999 --spec specs/439-intake-is-in-the-repository.md --title intake-without-a-daemon --criteria true && pii queue ls | grep -q 9999`

## Files

- `crates/pii-cli/src/cmd_queue.rs`
- `crates/pii-server/src/queue/` (the store the command writes)
- `scripts/autonomy/plan.tsv` (the rows added by the run that proves it)

## Guardrails

- Do NOT keep the helper "for the case where the daemon is down". That case is the norm here, and it is R2's job.
- Do NOT add the 085 path, or any other, to a tolerated list.
- Do NOT make the blocking-path check advisory for `scripts/autonomy/*` or `gate-steps.txt`.
- Do NOT touch the other session's file to make the tree clean.
- Before writing any further row against `pii queue`, read `cmd_queue.rs`: `add`, `ls` and `claim` already exist, and a
  spec describing them as missing is a spec that wastes a dispatch.