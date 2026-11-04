# Spec: pii writes the spec it will implement

**Project**: /projects/pi-improved · **Phase**: 0.2.1 · **Source**: user directive 2026-11-03 — *"pii should be able to
write specs (could use other model) and run it! the pii should suffice, in an autonomous way doing the whole flow"*.

## Verified premises (measured 2026-11-03)

- The **implement** half exists and works: `graphs/queue.toml` claims a row, spawns the workhorse on `$.nodes.select.item.spec`,
  and the judge runs the row's own `criteria`. Proven live tonight, after four defects were fixed
  (dead model id, missing spec, wrong claim path, no approval mode).
- The **produce** half does not exist: nothing in pii writes a spec file. `graphs/spec.toml:1` is "one tracked graph
  implements any spec" — it takes a spec, it does not author one.
- The queue now has a producer from outside (`pii queue add`, spec 352), so a written spec can be enqueued by a
  command. There is no node that enqueues.
- A planner model is configured and verified: `glm-5.3` on `ollama-cloud`, one-turn probe rc 0,
  `{"provider": "ollama-cloud", "model": "glm-5.3"}`. The workhorse is `deepseek-v4.1-flash` (user directive).
- The house spec shape is fixed and machine-read by two validators: `## Files` (one file per bullet, first backticked
  span) and `## Exit Criteria` (each line exactly `- [ ] \`command\``). Measured tonight: a spec that gets either
  heading wrong is refused by `row-paths.py` / `row-verify.py` with `NO-FILES` / `NOT-A-CRITERION`.

## Requirements

**R1.** A node kind `plan` that runs the **planner** role (not the workhorse) with a prompt built from: the finding or
case text, the repository path, and the house shape. It writes exactly one new file `specs/<NN>-<slug>.md`, where
`<NN>` is the next free number in the tree. It never overwrites an existing spec.

**R2.** The written spec must pass both validators with no human edit: `scripts/autonomy/row-verify.py check <NN>` and
`scripts/autonomy/row-paths.py check <NN>` exit 0 (which requires the row to exist — see R3).

**R3.** A node kind `enqueue` that takes a spec path and its title and calls the queue's producer with that row's
criteria read from the spec's `## Exit Criteria`, and the spec's files from `## Files`. It refuses a spec that fails
R2, naming the reason — a malformed spec must not become a dispatchable row.

**R4.** A composed graph `graphs/self-improve.toml`: `case → plan → enqueue → report`, using `call` for anything it
reuses and pinning any graph it calls. Its input is a case id or a finding text; its output names the spec it wrote
and the row it enqueued.

**R5.** The planner cannot write outside `specs/`: the node's capability set and workdir are the bound, and R1 is
checked by the node itself (a returned path outside `specs/` is a refusal).

## The requirement a plausible implementation satisfies in appearance only

A worker can add a `plan` node that asks the model "write a spec" and writes whatever comes back. The file exists, so a
naive test passes — and the row is unqueueable because the headings are prose, the criteria are sentences, and the
files list is a paragraph. The tests below run the two validators the lane itself uses, on a spec the node wrote,
with no human edit in between. That is the whole difference between a document and work.

## TDD contract

| file | test | assertion |
|---|---|---|
| `crates/pii-server/tests/plan_writes_a_spec.rs` | `a_case_becomes_a_spec_the_validators_accept` | given a finding text and a stub planner, one file appears under `specs/`, and both `row-verify.py check` and `row-paths.py check` exit 0 for its number |
| same | `CONTROL_two_runs_do_not_overwrite` | a second run takes the next number; the first file is byte-identical afterwards |
| same | `a_malformed_spec_is_refused_not_enqueued` | a stub planner that returns prose with no `## Files` → the `enqueue` node refuses, naming the missing heading, and the queue is unchanged |
| `crates/pii-server/tests/self_improve_composes.rs` | `the_flow_is_a_composition` | `pii graph lint graphs/self-improve.toml` exits 0; the graph calls `queue` by pin and invents no node kind |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test plan_writes_a_spec 2>&1 | grep -qE 'test result: ok. 3 passed'`
- [ ] `cargo test -q -p pii-server --test self_improve_composes 2>&1 | grep -qE 'test result: ok. 1 passed'`
- [ ] `grep -q 'kind = "plan"' graphs/self-improve.toml`
- [ ] `grep -q 'kind = "enqueue"' graphs/self-improve.toml`

## Guardrails

- The planner must NOT implement the spec it wrote; it writes, the workhorse implements, and a separate run judges.
- Do NOT let the enqueue node accept a spec that fails either validator (R3).
- Do NOT overwrite an existing spec file (R1).
- Do NOT give the planner `fs_write` outside `specs/`.
- The worker does not commit.

## Files

- `crates/pii-server/src/node_kinds/nodes/plan.rs`
- `crates/pii-server/src/node_kinds/nodes/enqueue.rs`
- `crates/pii-server/src/node_kinds/nodes/mod.rs`
- `crates/pii-server/src/engine/registry.rs`
- `graphs/self-improve.toml`
- `crates/pii-server/tests/plan_writes_a_spec.rs`
- `crates/pii-server/tests/self_improve_composes.rs`
