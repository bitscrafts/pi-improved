# Composition: what already exists, and the four gaps that do not

Filed 2026-11-02T18:35Z from an Opus plan-mode analysis run. **Claude Code + `claude-opus-5-5`** (probe answer
`modelUsage=claude-opus-5-5`, CLI 2.1.300), plan mode READ-ONLY, repo HEAD `79e426281973` (tree: the worker's abandoned
attempt, now moved to `_trash/343-attempt-20261102T182637Z`). 43 turns, $2.12. **STATUS: NOT a spec and NOT a plan
of record** — nothing here is authorised work; each proposal needs its own spec and its own gate.

Brief: `_tmp/pii-review-composition/brief.md`. Session: `9e081980-27ae-42d6-b07e-94eccce888c6`.

**Headline:** graph-includes-graph already ships as `kind = "call"` (spec 255); the job is to extend it, not to
build a second include mechanism. Specs 342 and 343 were amended accordingly.

---

# Design: "a graph includes a graph" in pii (specs 342/343): review of the abandoned attempt

All evidence comes from HEAD `79e4262` plus the dirty tree, read-only. Cargo is at `/usr/local/cargo/bin` because it is not on the default PATH.

## Context

Spec 343 asks for a new `graph` node kind that runs a pinned child graph as its own run. Spec 342 asks for a
night graph built only from kinds that already exist. A worker left a sketch of both. **Graph-includes-graph
already ships.** Spec 255's `kind = "call"` inlines a pinned graph at load. Specs 256, 260, 261, 269, 270 and 306
harden it, and `graphs/spec.toml:44-52` uses it in production. So the job is to amend 343 so it extends `call`, and
to close the real gaps: suspension and resume inside a call, failure attribution, output aliasing, and the
fixed-id lookups in the `queue` kinds. Building a second include mechanism is the wrong job.

---

## 1. Premises in this brief (and in the specs) that are wrong

| # | Premise | What the code says |
|---|---|---|
| P1 | "Fragments already compose at one level" is all that exists. Spec 342 says graph-includes-graph "is **not** supported today". | `crates/pii-graph/src/fragment/call.rs:1-17` (HEAD) is "Spec 255: a `call` node inlines another graph at load time". It has a sha256 pin (`:201-206`), a caps check (`:150-156`, `:242-256`), a cycle chain (`:157-162`) and a depth cap of 4 (`:33`). `graphs/spec.toml:44-52` calls `docs` by pin. `grep -rln 'kind *= *"call"' graphs crates` finds 13 files. |
| P2 | Spec 343: "the one kind this feature is allowed to add". | `graph` would be a second spelling of `call`. The worker's own diff proves it: `as_call()` renames `pin→sha256` and `input→in`, then sets `kind = "call"` (dirty `call.rs:402-427`). |
| P3 | "The child runs as its own run" is a premise of the design. | Spec 255 already took the opposite decision on the record. It cites an Opus design review that chose inline at load, giving "ONE graph: one run id, one journal, one cancel token, one crash-resume path" (`specs/255…md` Overview). Spec 343 does not cite that decision or argue against it. |
| P4 | "Capabilities have a ceiling (spec 215)". | This is half right. Spec 215 is titled "criteria are data not shell". Its item 3 adds the grant ceiling: `toml_load.rs:249` `check_grant_ceiling`, error `graph.grant.ceiling`. A call has two more layers on top: host→call (`call.rs:150`) and call→node (`call.rs:248`). |
| P5 | "4 passed; 4 failed", with three failure lines quoted. | The fourth failure is `graph_include.rs:131` `child_outputs_appear_under_the_node_id` ("output not mapped under node id"). The worker left `// TODO(spec 343): output … not yet mapped`. |
| P6 | `include_resolution` was not run. | `cargo test -q -p pii-graph --test include_resolution` gives **7 passed**. These tests exercise `include.rs` in isolation, and **nothing in the loader calls `include.rs`**: `grep -rn 'include::' crates` finds only the test. |
| P7 | `queue.toml` can be composed by the night. | Today it cannot, for three reasons. (a) It has no `exit`, so the call is refused with `graph.call.exit` (`call.rs:221-229`). (b) The `queue` kinds read the blackboard by fixed ids: `queue.rs:26` `nodes.get("select")`, `queue.rs:237` `nodes.get("judge")` and `nodes.get("dispatch")`, and `coder/helpers.rs:127` `.get("select")`. After namespacing to `sdd__select` they all read `None`. (c) It has absolute paths at `queue.toml:24,33`. `spec.toml` also has no `exit`. |
| P8 | Spec 343: "lint … names the file, **the line**". | The loader works on a `toml::Table`, which carries no spans. The worker faked line numbers: `include.rs:24-27` returns the line of the *first* `graph =` in the file, whichever include is at fault. |
| P9 | Spec 343: the cycle is printed with `→`. | The shipped message is ASCII `a -> b -> a`, and `call_inlines_a_graph.rs::an_indirect_cycle_is_refused_with_the_chain` asserts it. The worker made the arrow depend on the kind (dirty `call.rs:161-162`), so one rule now has two messages. |

## 2. Verdict on the abandoned attempt

| File | Verdict | Why |
|---|---|---|
| `crates/pii-graph/src/include.rs` | **DELETE (harmful)** | It is a parallel mechanism the loader never calls. `check_ceiling` gates only `PROC` and `SPAWN` (`:13`), so it is a weaker copy of the three existing cap layers. `check_cycle` (`:68`) runs over an edge list that nothing produces. `check_pin`'s line number is wrong whenever a file has more than one include (`:24-27`). Its tests pass and prove nothing (see trap T1). |
| `crates/pii-graph/src/lib.rs` (`pub mod include`) | **DELETE** (revert the line) | It only exists to export the file above. |
| `crates/pii-graph/src/fragment/call.rs` (diff) | **DELETE** (revert) | It turns `graph` into an alias of `call` with renamed keys. That is two spellings of one meaning. It also makes the cycle message depend on the kind and defaults a missing `caps` to `[]`, which silently undoes spec 255 R1 ("caps required"). And it pushes the file to 421 effective lines, against a 450 gate (`xtask lint-files`, run on the dirty tree), which leaves no room for the real work. |
| `crates/pii-graph/src/node.rs` (registers `bash`, `night_pick`, `night_land`) | **DELETE** (revert) | Spec 342's guardrail says "do NOT keep `night_pick`/`night_land` in any branch". |
| `crates/pii-graph/src/nodes.rs` (`pub mod …`) and `src/nodes/{bash,night_pick,night_land}.rs` | **DELETE (harmful)** | `bash.rs:55` runs `bash -c <cmd>` from a payload expression, which is the shell string spec 215 forbids. `night_land.rs:1-5` **makes git commits** under the identity `pii`, which breaks "a worker never commits" (`queue.toml:4`). `night_pick` duplicates `select`. |
| `crates/pii-graph/tests/night_nodes.rs` | **DELETE** | It tests the three kinds above. It also writes fixtures under the repo's `_tmp/` rather than a tempdir (`:12`). |
| `crates/pii-graph/tests/include_resolution.rs` | **DELETE** | It tests the orphan module. The real refusals are already covered by `call_inlines_a_graph.rs`. |
| `crates/pii-server/tests/graph_include.rs` | **REWORK → move** | The fixture idiom is worth keeping: `dirs()`, `pin()`, and `load()` through `load_host_with_graphs`. But it uses only `pii_graph`, so it belongs in `pii-graph`. The `kind="graph"` syntax should become `call`. The output test asserts on `format!("{:?}", spec)` (`:128-131`), which is appearance-only (T2). The three `panic!` stubs carry useful names but have no bodies. Keep the names and rewrite them as the tests in §6. |
| `graphs/night.toml` | **DELETE → rewrite** | It uses three new kinds (`:25,40,52`) and nine absolute paths (`:26-27,34,43,54-55,62`), which breaks spec 342's own NFRs. It does not compose `queue` at all. |

Clean-up must use `git checkout -- <tracked>` and `mv` of the untracked files into `_trash/`. The global rule bans `rm -rf`.

## 3. The design: extend `call`, add no kind

**Decision: inclusion stays at load time, flattened, with namespaced ids (spec 255's mechanism). Spec 343 is
amended to say `kind = "call"` and to drop "the child runs as its own run".** Here is how the two shapes compare
on each axis the brief asks about (Q1):

| Axis | Load-time inline (`call`, exists) | Child run (`graph` node) |
|---|---|---|
| Caps | All three layers are checked statically at load (`toml_load.rs:249`, `call.rs:150,248`). At run time each node gets root ∩ graph ∩ node (`sched/dispatch.rs:100`). | A run-time intersection would also need the parent ceiling passed into the child's start. A refusal can arrive mid-run. |
| Cycles | The `chain` stack (`call.rs:89,157,216-218`) refuses at load and allows diamonds. | A stack per child start is detected at run time, after side effects have happened. |
| Suspend / resume | Free. One run, one journal. Resume reads the **stored expanded spec** by hash (`engine/runs.rs:419-424`; written at `run/mod.rs:111-112`). The `max_entries` count is rebuilt on replay (`sched/state.rs:237`). | It needs parent↔child resume coordination, two journals, cancel propagation, and budget sharing ("an include does not reset them"). That is a new subsystem. |
| Journal | One journal. A `graph.call` event per call carries `call-from` (`run/call_events.rs:9-18`). | Two journals, and the incident scanner must join them. |
| Lint | `pii graph lint` = `check` → `load_host_with_graphs` + `validate` sees the whole composed graph (`engine/dispatch.rs:394`, `pii-cli/src/main.rs:283`). | Lint sees one level only. Child-of-child problems appear at run time. |

The child-run form wins only for a graph name chosen at run time. Spec 255 lists that as a follow-up, and neither
the night nor any graph today needs it.

```
 night.toml (host)                         load_host_with_graphs(src, fragments, dirs, reg)        [weave.rs]
 ┌──────────────────────────────┐            │ expand_host  (call.rs:71)
 │ clock(timer) → pick … → sdd  │──parse──►  │   for each kind="call": find → sha256 == pin? → parse
 │ sdd: kind="call" graph=queue │            │     chain.contains? (cycle) · depth ≤ 4 · caps ⊆ host · node caps ⊆ call
 │      sha256=… caps=[…]       │            │     call_needs::check · rewrite $.input/$.nodes (call_rewrite.rs)
 │      max_entries=288         │            │     splice: sdd__in → sdd__in__<k> → sdd__select → … → sdd__record (exit)
 │      [node.in] k="$.…"       │            │ NEW alias_outputs: host "$.nodes.sdd.<k>" → "$.nodes.sdd__record.<k>"
 └──────────────────────────────┘            │ weave [[attach]] → load_toml_with_registry → check_grant_ceiling
                                             ▼
                          ONE GraphSpec ──to_json──► store.put_graph (run/mod.rs:111) ──► header.graph
                          run: one journal; graph.call event at sdd__in; suspend at sdd__<n> = run Suspended
                          resume: GraphSpec::from_json(store[header.graph]); never re-reads graphs/*.toml
```

**Answers, Q2 to Q8**

- **Q2 Identity.** There is one run id. Child nodes are `<call>__<n>`, and nesting gives `outer__inner__n`. A
  collision is refused as `graph.call.id` (`call.rs:335-344`). The parent names the child through the `graph.call`
  NodeObserved on `<call>__in`, with fields from `CallRecord` (`fragment/call_record.rs:12-29`: call, graph,
  sha256, caps, needs, exit, input_keys, parent). Spec 343's "child run id + failing node id" becomes the
  namespaced failing node id, e.g. `sdd__judge`. That id names the include and the inner node, and the journal
  maps `sdd` → graph + sha.
- **Q3 Inputs and outputs.** The mapping syntax is `[node.in] key = "<literal or $.expr>"`. Each key becomes the
  transform node `<call>__in__<key>`, and the child's `$.input.key` is rewritten to `$.nodes.<call>__in__<key>`
  (`call_rewrite.rs:35-37`). Keys the child declares in `needs` and the call leaves unmapped read the host input
  (spec 261, `call_rewrite.rs:38-44`). For output, today the host reads `$.nodes.<call>__<exit>.<k>` and routes on
  `from = "<call>"` ports (`call.rs:346-358`). **Add:** rewrite host expressions `$.nodes.<call>.<rest>` →
  `$.nodes.<call>__<exit>.<rest>`, so the host never names the child's internals. **Does the child see the
  parent's blackboard?** Not through `$.` expressions: `call_needs` refuses host-node reads (`call_needs.rs:1-3`).
  But it does through Rust: `NodeInput.input` and `NodeInput.nodes` are the host's (`call.rs:9-12`,
  `post.rs:111`), and the fixed-id lookups (P7) depend on exactly that leak. It should not: retire the fixed-id
  lookups (step 4).
- **Q4 Caps.** Three load-time refusals: `graph.call.widen` for a call wider than its host (`call.rs:150-156`),
  `graph.call.widen` for a node wider than its call (`call.rs:248-255`), and `graph.grant.ceiling` against the
  daemon (`toml_load.rs:249,274`). They are refusals, not silent intersections. A silent intersection would turn
  an escalation into a run-time `Denied` deep in a child. A child that asks for less is accepted
  (`a_call_asking_no_caps_loads`). Its nodes run with root ∩ graph ∩ node (`dispatch.rs:100`), so they get less.
- **Q5 Cycles.** The chain starts as `[host name]` (`call.rs:89`) and is pushed and popped around each sub-load
  (`:216-218`). A self-include prints `graph.call.cycle: a -> a`; a two-graph cycle prints `a -> b -> a`. Both are
  already tested. **Add** the call id, so an operator can find the line:
  `graph.call.cycle: a -> b -> a (call 'back' in graph 'b')`.
- **Q6 Suspend and resume.** A timer or gate inside a call is the run's own suspension, keyed to `sdd__<n>`.
  - A finished child's nodes are journalled done and are not re-dispatched on replay.
  - A suspended child is resumed at the suspended namespaced node (`run/resume.rs:143-178`).
  - A failed child ends the run `Failed`, and resume is refused with "run already finished" (`engine/runs.rs:415`).

  **The case a naive design gets wrong:** in a child-run design, resuming the parent re-dispatches the include
  node, which starts a *new* child instead of resuming the suspended one. Its side effects (coder, post, record)
  then run twice. In the inline design the equivalent trap is the cumulative `max_entries` counter (default 8,
  `call.rs:36`, rebuilt on replay at `state.rs:237`). The night re-enters `sdd` on every tick, so it fails on the
  9th row unless `max_entries` is set.
  **Unverified:** no test today suspends *inside* a call (I checked: the only call tests that mention gates are
  `call_max_entries.rs`, which uses a counting gate and no suspension). That is the first test to write.
- **Q7 Pins.** The run start stores the **expanded** spec, content-addressed (`run/mod.rs:111-112`), and resume
  deserialises that blob (`runs.rs:419-424`). So editing `queue.toml` while a night is suspended has no effect on
  that run. The pinned copy is already in the store, and resume is deterministic. The next fresh start or lint
  refuses with `fragment.pin.mismatch` naming both digests (`call.rs:202-205`). A pinned copy is required, and it
  already exists. A hash check on resume would be worse: it would refuse an in-flight night because someone edited
  a file. No change is needed; add a test that locks the behaviour in.
- **Q8 (342).** Yes, the night becomes short: `clock(timer) → sdd(call queue) → loop`, plus `none → review → post`,
  with `[[cron]]`. But `queue.toml` **cannot be reused verbatim**. It needs, in this order: `exit = "record"`,
  a `needs` list, paths moved into input (`workdir = "$.input.workdir"`), and the queue kinds reading their peers
  through config expressions instead of fixed ids. These are edits to queue.toml itself, which changes its pin,
  and nothing pins it today. `spec.toml` needs `exit = "done"` and `needs = [...]`. Note that its `refuse` nodes
  end the **whole** run as `Refused` rather than routing to an exit port. `discord-digest.toml`'s `post` is used
  as a kind (not a call), since its template carries a literal channel. `select` emits `none`? **Unverified.**
  `queue.rs:48` `claim_next` decides it; check its ports before wiring `pick none → review`.

**Types and functions introduced** (no new trait, no new kind):

- `crates/pii-graph/src/fragment/call_output.rs` (new, about 60 lines, keeps call.rs under the 450 gate):
  `pub(super) fn alias_outputs(doc: &mut Table, call: &str, exit: &str)`. It walks host node values (not
  `id`/`kind`) and rewrites `$.nodes.<call>` and `$.nodes.<call>.<rest>` to `$.nodes.<call>__<exit>…`. Call it
  from `splice` after the host edge rewrite (`call.rs:346-358`).
- `call.rs:157-162`: add the call id to the cycle message. This is a format change only.
- `crates/pii-server/src/node_kinds/nodes/queue.rs:25-27,237` and `coder/helpers.rs:121-128`: read
  `cfg.item` / `cfg.verdict` / `cfg.dispatch` as `eval_or_literal` expressions, **defaulting to today's fixed
  paths when absent** (the control). `queue.toml` then states them explicitly, so `call_rewrite` namespaces them.

## 4. Ordered implementation steps (one commit each; each passes `cargo run -q -p xtask -- fresh-gate`)

1. **Revert the abandoned attempt.** Run `git checkout --` on the four tracked files, and `mv` the six untracked
   paths to `_trash/343-attempt-<ts>/`. *Observable:* `git status` is clean, and
   `cargo test -q -p pii-graph --test call_inlines_a_graph` gives all pass (the baseline).
2. **Amend specs 343 and 342** (no code). 343 uses `kind="call"`, drops the child run, keeps the `->` arrow and
   names the call id instead of the line. 342 removes "not supported today" and names `call` as the reuse
   mechanism. *Observable:* the spec diff only.
3. **Tests first: suspension and failure inside a call** (`pii-graph/tests/call_suspends_and_resumes.rs`). If they
   pass with no code change, the gap was only a missing test, and that is recorded. *Observable:* the 6 tests in §6
   group A.
4. **The `queue` kinds read their peers through config expressions**, with fixed-id defaults. Tests go first
   (§6 group C). *Observable:* `queue.toml` run standalone gives the same verdict as before (the control), and run
   under a call it records the item.
5. **Make `queue.toml` callable:** `exit`, `needs`, and paths from input. Adjust `spec.toml` the same way.
   *Observable:* `pii graph lint graphs/queue.toml` passes, and a host that calls it by pin lints.
6. **Output alias** `$.nodes.<call>.<k>` (`call_output.rs`) and the call id in the cycle message. *Observable:*
   §6 group B.
7. **Rewrite `graphs/night.toml`** as a composition, plus `crates/pii-server/tests/night_graph_composes.rs`
   (§6 group D). *Observable:* `pii graph lint graphs/night.toml`, and
   `pii run start night --input '{"dry_run":true}'` reaches `review` on an exhausted queue.
8. **Daemon-level resume across a restart and a child edit** (`pii-server/tests/call_across_restart.rs`,
   §6 group E).

## 5. Traps, ranked

- **T1. An orphan checker with green tests.** `include.rs` plus `include_resolution.rs` give 7 passing tests while
  the loader never calls them. Every refusal "works", and no graph is composed. *Caught by* tests that go through
  `load_host_with_graphs` or a run, never a bare helper.
- **T2. The fixed-id leak.** The night calls `queue`, lints, and runs. Then `sdd__record` finds no `select` and
  fails ("record: no item was selected", `queue.rs:229-233`). If the host has its own node named `select`, it
  silently reads the **host's** item instead. *Caught by* `queue_kinds_read_their_peers_inside_a_call`.
- **T3. `max_entries` = 8 on a looped call.** The night dies on its 9th row, and the counter survives a resume.
- **T4. Asserting on Debug text.** `graph_include.rs:128` greps `format!("{:?}", spec)` for `$.nodes.sdd.verdict`.
  The string can appear while resolving to `null` at run time. *Caught by* running the graph and reading the
  terminal value.
- **T5. A Rust-side `NodeInput.input` read** inside a child sees the host input (`call.rs:9-12`). `coder`'s
  `scope_root`, `heal_record` and `post`'s template context all do this. They are invisible to lint.
- **T6. A `refuse` inside a called graph ends the whole host run**, not just the call. `spec.toml`'s
  `unverified`/`refused` would end the night. The night must call a graph whose failures leave on exit ports, or
  accept that one bad row stops the night.
- **T7. Re-reading files on resume** "for safety". This makes a resume non-deterministic and lets an edit refuse
  an in-flight run.

## 6. The test set

**A. `crates/pii-graph/tests/call_suspends_and_resumes.rs`** (builtin `timer`; in-memory store):

- `a_timer_inside_a_call_suspends_the_host_run`: status is Suspended, and the resume key's node is `sub__tick`.
  The **control** is `CONTROL_the_subgraph_alone_suspends_at_tick`.
- `resuming_the_host_continues_inside_the_call_at_the_suspended_node`: after the fire, `sub__after` and then
  `done` run. A counting node before the call has a run count of exactly 1, and `sub__in` is dispatched once.
- `a_call_that_finished_before_the_suspend_is_not_rerun`: the timer sits *after* the call, and the counting node
  inside the call has a run count of 1 after resume.
- `a_failure_inside_a_call_names_the_call_and_the_node`: the run fails, the error or terminal contains `sub__boom`,
  and the journal's `graph.call` names graph + sha. The **control** is
  `CONTROL_a_host_failure_names_the_host_node`.
- `the_entry_count_survives_a_resume`: suspend after 7 entries with `max_entries = 8`, then resume. The 8th entry
  passes and the 9th fails. The **control** is the existing `call_max_entries` tests.
- **Fails if reverted?** These pin existing behaviour. They fail if anyone introduces a child run or re-entry on
  resume, which is their purpose. State that in the test header.

**B. `crates/pii-graph/tests/call_outputs_and_cycles.rs`** (the reworked `graph_include.rs` names):

- `child_outputs_appear_under_the_call_id`: **run** the graph. A host node `$.nodes.sdd.verdict` evaluates to
  `"approved"`. It fails if `call_output.rs` is reverted (it resolves to `null`). The **control** is
  `CONTROL_the_exit_id_still_resolves` (`$.nodes.sdd__judge.verdict`).
- `a_host_without_the_alias_is_byte_identical`: a host that never reads `$.nodes.<call>` loads to an equal spec
  before and after the change.
- `a_cycle_names_the_call_that_closes_it`: the error contains `a -> b -> a` and `call 'back'`. The **control** is
  the existing `an_indirect_cycle_is_refused_with_the_chain`, which keeps its assertion.
- The unpinned, mismatch and caps cases already exist and are kept unchanged as regression:
  `a_call_without_sha256_is_refused`, `a_wrong_pin_is_refused_with_both_digests` + `a_correct_pin_loads`,
  `a_subgraph_cannot_widen_caps` + `a_subgraph_within_the_grant_loads`, and
  `a_call_widening_the_host_is_refused_at_the_call_site` + `a_call_asking_exactly_the_host_grant_loads`
  (`call_inlines_a_graph.rs`). The lint-level check is the existing `pii-cli` test
  `check_refuses_a_wrong_pin_naming_both_digests`.

**C. `crates/pii-server/tests/queue_in_a_call.rs`** (the `queue` kinds live in pii-server):

- `queue_kinds_read_their_peers_inside_a_call`: seed one queue item and run a host that calls `queue` by pin. The
  item's status is Done or Failed with a verdict, **not** "no item was selected". It fails if step 4 is reverted.
- `CONTROL_queue_toml_alone_records_the_same_verdict`.
- `a_host_select_node_is_not_read_by_the_called_record`: the host has its own `select` with a different item.
  The called `record` writes the *called* item.

**D. `crates/pii-server/tests/night_graph_composes.rs`** (spec 342's four tests, plus):

- `night_calls_queue_at_its_current_pin`: the `CallRecord` for `sdd` has `graph == "queue"` and `sha256 ==
  sha256(graphs/queue.toml)`.
- `night_graph_uses_only_existing_kinds`, `night_graph_kinds_are_a_subset_of_the_graphs_it_composes`,
  `night_graph_carries_no_absolute_project_paths`, and `night_stops_on_exhaustion`.
- `night_call_sets_max_entries_at_least_its_loop_budget` (T3).

**E. `crates/pii-server/tests/call_across_restart.rs`:**

- `a_suspended_call_resumes_after_a_daemon_restart`: a gate inside the call. Drop the engine, reopen the store,
  resume, and the run finishes.
- `editing_the_called_graph_does_not_change_a_suspended_run`: overwrite the child file mid-suspension. The resume
  runs the **pinned** value. A fresh start then refuses with both digests. The **control** is the unedited run.

## 7. What I would NOT build

- **A `graph` kind, or `graph` as an alias of `call`.** It is two spellings of one meaning, and spec 342's own
  guardrail ("keep one meaning") rules it out.
- **A child run per include.** It would need a new resume, cancel, budget and journal subsystem, and the only
  thing it buys is a run-time graph name that nobody needs yet.
- **`output = [...]` filtering.** The host can still read the exit's whole value through `$.payload`, so the
  filter would promise isolation it cannot enforce. The alias gives the ergonomics without the false promise.
- **Line numbers in load errors.** That needs a span-carrying TOML parse across the whole loader. The call id is
  unique per file and is enough to find the line.
- **Intersecting caps silently.** Refusing at load is strictly more informative.
- **The `bash` node.** Criteria are data (spec 215), and `judge` already runs a row's argv.

## Verification

- `export PATH=/usr/local/cargo/bin:$PATH`.
- After each step: `cargo test -q -p pii-graph`, `cargo test -q -p pii-server`, `cargo test -q -p pii-cli --test graph_commands_expand_calls`, `cargo run -q -p xtask -- lint-files`, and finally `cargo run -q -p xtask -- fresh-gate` in a clean clone.
- End to end: `pii graph lint graphs/night.toml`; `pii run start night --input '{"dry_run":true}'` on an exhausted queue reaches `review`, not `sdd__dispatch`; and `pii graph explain night` prints the `sdd → queue @ sha` call line.
- Revert check: for each group-B/C test, revert its step locally and confirm that the test fails.
