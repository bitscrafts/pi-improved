# Spec: A resumed row may resume its own work

**Project**: <repo> · **Measured**: 2026-11-03, row 359.

## Why this exists

Row 359 had work in the tree — eight files it owns (`crates/pii-cli/src/cli_util.rs`, `cmd_daemon.rs`,
`cmd_entity.rs`, `cmd_index.rs`, `cmd_lane/mod.rs`, `cmd_learn.rs`, `cmd_skill.rs`,
`crates/pii-server/src/engine/registry.rs`). The lane resumed its session and the dispatcher answered:

```
STOPPED: dirty tree (set ALLOW_DIRTY=1 only to resume this spec's own WIP):
 M crates/pii-cli/src/cli_util.rs
 … 8 files
```

`worker-rc=2` in the row's state. The refusal is **correct by design** — a dispatch must not start on a tree it did
not author. But the dirty paths were *identical* to the row's own declared paths, which is precisely the case the
script's own message calls out ("this spec's own WIP"), and the lane never checked before refusing.

So a row that legitimately has its own uncommitted work cannot be resumed by the lane at all; a person has to notice
and pass `ALLOW_DIRTY=1` by hand. That is the same family as the park that wedged the queue (row 361): a safety
rule firing on the case it was written to permit.

## Requirements

- **R1** Before refusing a dirty tree, the dispatcher compares the dirty paths with the row's declared paths
  (`row-paths.py`): if **every** dirty path is in the row's list, it proceeds and says so, naming the count.
- **R2** If **any** dirty path is outside the row's list, it still refuses, and the refusal names the offending
  paths — not the whole list.
- **R3** The decision is printed and recorded in the row's state (`last=`), so a refused dispatch is visible without
  reading the log.
- **R4** `ALLOW_DIRTY=1` keeps working as the manual override for the case R2 refuses.

## Files

- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/cycle.sh`
- `scripts/checks/tests/lane-own-wip.test.sh`

## Exit Criteria

- [ ] `bash scripts/checks/tests/lane-own-wip.test.sh 2>&1 | grep -q 'RESULT: 0 failed'`
- [ ] `grep -q 'row-paths' scripts/run-batchC-pii.sh`
- [ ] `grep -q 'outside this row' scripts/run-batchC-pii.sh`

## Guardrails

- Do NOT weaken the refusal: a path outside the row must still stop the dispatch.
- Do NOT let a dirty tree become acceptable in general; the exception is exactly "all of it is this row's".
- Do NOT guess ownership from the session id; the row's declared paths are the only source.
- Test with the near-miss control: one file outside the row must still refuse.
