# Spec: A row can be put in the queue from outside

**Project**: /projects/pi-improved · **Phase**: 0.2.1 hardening · **Unblocks**: the night run, and every later
self-improvement row (a case must become work somehow)
**Finding**: measured 2026-11-03 — `put_item` is called **only from tests**

## Verified premises (measured 2026-11-03)

- `grep -rn 'put_item' --include='*.rs' crates/` returns five hits: `queue_graph_selects.rs:23`,
  `queue_graph_pipeline.rs:107,144,186`, `queue_blocked_is_not_empty.rs:23`, and one unit test inside
  `crates/pii-server/src/node_kinds/nodes/queue.rs:428`. **No production caller exists.**
- `graphs/queue.toml` is `select → coder → judge → record`. The `select` node calls
  `self.store.claim_next("select")` (`node_kinds/nodes/queue.rs`), and the coder takes `workdir`/`spec` from the
  claimed row, while the judge runs the row's own `criteria`.
- `pii-store/src/queue.rs:50-61` — `WorkItem { id, spec, title, criteria, touches, … }`.
- `pii queue` does not exist: `crates/pii-cli/src/main.rs` dispatch has `run`, `cron`, `soak`, … but no `queue`.
  Today `pii queue add …` exits 2 with the usage line.
- The store is opened by the **daemon** (`pii-server`); the single-writer rule means the CLI must talk to the
  daemon over the control socket, never open the redb file itself.

## Requirements

**R1.** A daemon method `queue.add` that takes `{id, spec, title, criteria[], touches[]}` and writes one
`WorkItem` through the existing `QueueStore::put_item`. It returns the stored item's id.

**R2.** `queue.add` refuses — with a typed error naming the reason — when:
(a) `id` is already in the queue in any state (pending/claimed/done/failed) — a re-seed must not duplicate work;
(b) `spec` is a path that does not exist on the daemon's machine — a row pointing at nothing wastes a coder pass;
(c) `criteria` is empty — the judge would have nothing to run, and a row that cannot fail is not work.

**R3.** A daemon method `queue.list` returning `{id, state, title, spec}` per row, oldest first.
**R4.** A daemon method `queue.claim` with `{by}` returning the claimed item or `null` — the same `claim_next`
the graph's `select` node uses, exposed so a dispatcher (Hermes cron or a test) can drive the lane without a graph.
**R5.** The CLI: `pii queue add --id <id> --spec <path> --title <text> --criteria <cmd> [--criteria <cmd>]…
[--touches <path>]…`, `pii queue ls`, `pii queue claim --by <name>`. Each is one RPC; none opens a store.
**R6.** Every successful add writes an observe event named `queue.item.added` carrying the id (the lint's
`lint-observability` budget applies: new code uses `writeln!`/the observe seam, not raw prints).

## The requirement a plausible implementation satisfies in appearance only

A worker can add the RPC, the CLI and a unit test that calls `put_item` directly and asserts the row is there.
That proves nothing about the lane: the defect is that **the daemon's queue can be fed by something other than a
test**. The tests below go through the CLI → socket → daemon → store, and the last one proves a graph consumes
what the CLI put in.

## TDD contract — `crates/pii-cli/tests/queue_add.rs`

| test | setup | assertion |
|---|---|---|
| `a_row_added_from_the_cli_is_visible_to_the_daemon` | scratch daemon; `pii queue add --id W1 --spec <existing file> --title t --criteria 'true'` | rc 0; `pii queue ls` lists `W1` as pending |
| `CONTROL_adding_the_same_id_twice_is_refused` | add `W1`, add `W1` again | second rc 2 and the error names `W1`; `pii queue ls` still shows exactly one row |
| `CONTROL_a_row_whose_spec_does_not_exist_is_refused` | `--spec specs/does-not-exist.md` | rc 2, error names the path |
| `CONTROL_a_row_with_no_criteria_is_refused` | no `--criteria` | rc 2, error says the judge would have nothing to run |
| `a_row_added_from_the_cli_is_claimed_by_the_select_node` | add a row, then run the `graphs/queue.toml` `select` node against the same daemon | the run reports `queue.item.added` then a claim of that id; the coder is **not** reached (stub the coder kind or use `pii graph lint` + a claim-only run) |

All five must be RED before the change (the CLI has no `queue` subcommand, so the first four fail on rc 2 anyway;
say so in the Closure and paste the output).

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test queue_add 2>&1 | grep -qE 'test result: ok. 5 passed'`
- [ ] `cargo run -q -p pii-cli --bin pii -- queue add --help 2>&1 | grep -q -- --criteria`
- [ ] `grep -q 'queue.item.added' crates/pii-observe/src/names.rs`
- [ ] `grep -q 'Method::QueueAdd' crates/pii-server/src/engine/dispatch.rs`
- [ ] `grep -q 'fn cmd_queue' crates/pii-cli/src/cmd_queue.rs`
- [ ] `[ "$(grep -c 'put_item' crates/pii-server/src/engine/dispatch.rs 2>/dev/null)" -ge 1 ]`
## Guardrails

- The gate is the cycle's step, not the implementer's. `cycle.sh` gates this commit with the full step list and a
  measured coverage floor (specs 262/266/315); a sandboxed implementer cannot write `/projects/_tmp`, where
  `fresh-gate` exports its tree, so naming the gate as an exit criterion makes the run refuse before its work is
  judged (measured 2026-11-03, row 351: three attempts, 4 of 5 criteria green, parked on the gate alone). Do not
  add it back as a criterion.

- Do NOT let the CLI open a store: single writer, the daemon owns it.
- Do NOT make `add` overwrite an existing id (a re-seed must be refused, not silently replace).
- Do NOT add a second queue. `pii-store/src/queue.rs` is the queue; the autonomy JSON (`scripts/autonomy/state.py`)
  stays the builder lane's queue and is not touched here.
- Do NOT weaken `queue_blocked_is_not_empty` or the existing queue tests; their counts must not drop.
- The worker does not commit.

## Files

- `crates/pii-proto/src/lib.rs` (method enum + payloads for `queue.add`, `queue.list`, `queue.claim`)
- `crates/pii-server/src/engine/dispatch.rs` (the three handlers)
- `crates/pii-server/src/engine/dispatch.rs` (the three handlers, beside the existing ones)
- `crates/pii-observe/src/names.rs` (`queue.item.added`)
- `crates/pii-cli/src/main.rs`, `crates/pii-cli/src/cmd_queue.rs` (new)
- `crates/pii-cli/tests/queue_add.rs` (new)
