# Spec 328: one ledger — the flow's item ids are their work's spec numbers

## Overview

The same unit of work carried a queue row (`A187`), a spec (`specs/325-…md`) and a flow item (`A186` for spec
322). Specs 326 and 327 made the queue side name its work: from spec 326 on a row's id **is** its spec number,
and a row carries `spec` and `commit` as proven fields. The flow side was still a second counter:

```
.minilam/items/A184.json  -> data.spec: specs/320-…md   (done, history 15)
.minilam/items/A186.json  -> data.spec: specs/322-…md   (done, history 11)
```

They agreed with the queue only because a person typed matching names, and `A186` had no queue row at all. The
alias was recoverable in both cases — `data.spec` was already there — so the migration is a rename, not a guess.

## Requirements

**R1 — an item's id is its work's number.** For every item in the minilam store: the file name equals the
`id` field, and that id is the number at the front of `data.spec` (`specs/320-…md` → `320`).

**R2 — a legacy id must declare itself.** An `A…` name is legal only when the item carries `data.alias` equal to
that name. History stays visible; it does not stay a second identity.

**R3 — an item is tied to a spec that exists.** `data.spec` must name a file that is there, and it must start
with a number. An item with no `data.spec` is an item nobody can tie to any work.

**R4 — migration is a move, never a delete.** `item-ids.sh --migrate` writes the new file (carrying the
history), records `data.alias`, **moves** the old file into `<workspace>/_trash/<date>-item-ids/`, and repoints the
pointer when it names the old id. A collision with an existing item is refused and nothing moves.

**R5 — the gate holds the rule.** `item-ids|bash scripts/autonomy/item-ids.sh --check` is a gate step. The
minilam store is untracked project state, so a fresh-gate export has nothing to inspect and the step says so
instead of pretending it validated something.

**R6 — what this does NOT do, and why.** The autonomy cycle does not create flow items at dispatch. An item
that is created and never advanced would be a dead row in a second ledger — worse than no row. Advancing it
would mean mapping the cycle's phases (dispatch, verify, gate, commit) onto the method's states, which is a
change to the kit's model of the work, not orchestration glue. That decision is open, and the queue's own
fields already carry what the flow would say: `status`, `attempts`, `session`, `spec`, `commit`.

## Files

- `scripts/autonomy/item-ids.sh` (new: `--check`, `--migrate`)
- `scripts/autonomy/tests/item-ids.test.sh` (new)
- `gate-steps.txt` (the `item-ids` step)
- the store itself: `.minilam/items/{320,322}.json`, pointer `current: 322`

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| --check is red on a legacy id | item `A5` with `data.spec` for spec 900 | rc 1, names 900 |
| --migrate renames it | `--migrate` | `900.json` exists, `A5.json` gone from the store |
| it was MOVED, not deleted | the trash dir | `A5.json` is there |
| the history came across | migrated item | 1 history entry |
| the alias is declared | migrated item | `data.alias == "A5"` |
| the id field follows the name | migrated item | `id == "900"` |
| the pointer was repointed | pointer naming `A5` | `current == "900"` |
| --check is green after the migration | `--check` | rc 0 |
| a name/id disagreement is red | file `901.json` whose id says `A7` | rc 1 |
| a missing spec is red | item with `data: {}` | rc 1 |
| a collision is refused | `A9` whose spec's number is taken | rc 1, `A9.json` still in place |

## Exit Criteria

- [ ] `bash scripts/autonomy/tests/item-ids.test.sh 2>&1 | grep -qE "[0-9]+ ok, 0 FAIL"`
- [ ] `bash scripts/autonomy/item-ids.sh --check`
- [ ] `grep -q "^item-ids|bash scripts/autonomy/item-ids.sh --check" gate-steps.txt`
- [ ] `bash scripts/autonomy/run-tests.sh 2>&1 | tail -1 | grep -qE "[0-9]+ passed, 0 failed"`
- [ ] `minilam items | grep -qE "^\*? *322 "`
- [ ] `python3 -c "import json;d=json.load(open('.minilam/items/322.json'));assert d['id']=='322' and d['data']['alias']=='A186' and len(d['history'])==11"`
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree 2>&1 | tail -1 | grep -q "fresh-gate: PASS"`

## Live migration (2026-10-31)

```
--check   ITEM-IDS FAIL: A184: should be 320 …   A186: should be 322 …   rc=1
--migrate pointer moved A186 -> 322 · A184 -> 320 · A186 -> 322 (old files moved to _trash/20261031-item-ids)
--check   2 item(s) in step, 0 migrated, 0 violation(s)   rc=0
minilam items        320  done  history 15      * 322  done  history 11
```

## Guardrails

- Do NOT delete the old item file: move it, so a wrong migration is reversible.
- Do NOT rename an item whose `data.spec` is missing — the number would be invented.
- Do NOT repoint the pointer to an id that does not exist.
- Do NOT create flow items the cycle will not advance (R6): a second ledger of dead rows is worse than none.
