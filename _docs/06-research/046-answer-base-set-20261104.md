> Derived by the reviewer (read-only, `--permission-mode plan`) in answer to the brief beside this
> file. Committed by the orchestrator unchanged. Nothing here is ratified: the operator rules on it.

# 046: one minimal set of node kinds that expresses every graph

## Context

This answers `_docs/06-research/046-base-set-derivation-brief.md`. It is a paper derivation and nothing in the repo was changed.

- **Evidence used:** I parsed `[[node]]` tables, ran `target/debug/pii graph lint` on all 32 files, and read the code. The lint binary is from 2026-11-04 12:52.
- **Not done:** no rewritten graph has been executed, and the `let`/`out` extension to `transform` does not exist yet. Claims I could not confirm are marked **unverified**.

## Verdict

**The base set is 14 kinds for the capabilities, plus one test seam that decides test 1.**

| # | Test | Result | Count |
|---|---|---|---|
| 1 | Every shipped graph is expressible | **FAIL at 14, PASS at 15** | 31/32 at 14. `starter/crash` exists to panic a node, and no kind in the 14 can panic. |
| 2 | Nothing in the set is derivable | **PASS** | 14/14 sentences in A.2. `ledger` and `recall` are the weakest, and the case for each is stated there. |
| 3 | Nothing outside the set is needed | **PASS at 15** | 23/23 kinds have a row in B.2. 22 are replaced; `debug_crash` is only replaced by moving its graph out of `graphs/`. |
| 4 | `tool` cannot carry the proof | **PASS** | All 8 `tool` nodes in 4 graphs are re-expressed with no shell. This forces `judge` into the set and needs one widening of it. |
| 5 | Everything is a graph | **PASS** | `pii docs stale` is derived in C.4, and the rule is in C.5. |

**The smallest set that passes all five as written has 15 kinds: the 14 plus `debug_crash`.**

**My recommendation is 14.** Move `starter/crash.toml` to a test fixture and register its factory only for that test. A set whose purpose includes "a node can panic" is worse than a library of 31 files. Moving it is your decision, because it changes what test 1 counts.

The worry that eleven kinds would fail test 4 was justified. Eleven is reachable by making every store operation a typed tool, and A.3 explains why that puts the proof back on `tool`.

### Corrections to the brief

| Brief says | Measured |
|---|---|
| Registered in `node_kinds/registry.rs` | That file does not exist. It is `crates/pii-server/src/engine/registry.rs:25` (daemon) and `:205` (lint), and the two lists are separate. |
| Two files do not load | **Five files fail `pii graph lint`.** `code-index` and `intervention-report` exit 2 with `missing 'name'`. `tdd`, `sdd` and `night` exit 3 with V9, because `tdd.toml`'s `not-red`, `unrunnable` and `still-red` use `join = "any"` with one inbound edge. `validate` was only found on the lint paths (`engine/dispatch.rs:405`, `cmd_graph_installed.rs:274`), so those three may still start (**unverified**). |
| `recall`/`memory` are guarded by `proc` | Neither is. `recall` requires `MEMORY_READ` (`entity.rs:103,318`). `memory` requires nothing (`memory.rs:37`), and neither do `select` and `record` (`queue.rs:96,327`) or `heal_*` (`heal.rs:168,296`). |
| 23 kinds with those counts | Confirmed as files per kind. In nodes the library has 146, of which `transform` is 43. |
| `call` is one of the kinds | It is, but it has no factory. It is expanded at load (`fragment/call.rs:3`) and has no runtime. |

---

## A. THE BASE SET

### A.1 The table

| Kind | Semantics | Touches | Capability |
|---|---|---|---|
| `transform` | Pure value computation: path, literal, or `let`/`out` (section C) | nothing | none |
| `route` | Takes one port by comparing a value | nothing | none |
| `refuse` | Fails the run by name, or returns the verdict to a host that wired it | the run's outcome | none |
| `call` | Inlines a sha256-pinned graph at load | nothing at runtime | the host's |
| `timer` | Suspends durably until a time or cron fire | the clock, the wheel | none |
| `gate` | Suspends durably until a human decides | the approval store, a human | none |
| `post` | Sends one idempotent message to a channel | an external channel | `post` |
| `tool` | One entry from a closed surface: `fs_read`, `glob`, `grep`, `fs_write`, `run` (literal argv0, no shell) | files, one declared process | declared per node: `fs_read`, `fs_write`, `proc` |
| `judge` | Runs a spec's criteria as read from a pinned commit | processes the graph does not name | `proc` |
| `coder` | Spawns an external agent with an exact env, bounded and charged to the run | a process, a container, a model, the budget | `spawn`, `proc` |
| `agent` | In-process model loop whose tool calls go through `Ctx` | a model; files and processes only through listed tools | as listed; `net` for a live provider |
| `recall` | Reads the memory store (entities, documents) | memory store | `memory_read` |
| `remember` | Writes the memory store: `entity`, `relationship`, `doc`, `consolidate` | memory store | `memory_write` |
| `ledger` | One transaction on the work ledger: `queue.claim`, `queue.set_status`, `queue.put`, `heal.record_run` | queue and heal stores | none today; a new bit is recommended (below) |

Of the 14, 12 already ship. `remember` is registered but no shipped graph uses it (`registry.rs:118`). `ledger` is the only new name, and it replaces parts of five kinds.

**Honest vocabulary count:** 14 kinds, plus 4 `ledger` ops and 4 `remember` ops, plus 5 tool entries. I count ops openly because hiding them is exactly how a smaller number gets faked.

**The `ledger` capability:** `select`, `record` and `heal_record` hold no capability, so any graph can claim a queue row today. The fix is one bit in `crates/pii-abi/src/cap.rs`. That is outside the node kinds, so it is your call. The set works without it, at today's level of exposure.

### A.2 Why each one cannot be derived (test 2)

- **`transform`:** every other kind either has an effect or only selects, so nothing else can build a new value.
- **`route`:** the scheduler only kills edges by port (`state.rs:185`), and only `route` takes its port set from config and chooses by value.
- **`refuse`:** it is the only control kind that returns `Outcome::Fail` (`nodes.rs:294`), so without it a verdict could only be expressed as a run that succeeded.
- **`call`:** composition by pin happens at load, before any node exists, so no runtime kind can produce it.
- **`timer`:** only `timer` returns `Suspend` with a time reason (`nodes.rs:390`), and a blocking wait does not survive a restart.
- **`gate`:** a human's decision is not computable, and the decision is read from a store so that resuming after a restart is deterministic (`gate.rs:89`).
- **`post`:** with no shell there is no way to reach a channel, and the channel hub with its idempotency key belongs to the daemon.
- **`tool`:** nothing else reads a file or runs a named program under `fs_*`/`proc` with the command visible in the graph.
- **`judge`:** it is the only legitimate way to run a command the graph does not name, because its authority comes from the commit rather than the graph (`judge_spec.rs:3`). A restricted `tool` must refuse exactly that.
- **`coder`:** exact-env spawn, the per-kind argv grant (`node.rs:56`) and charging the run budget (`coder.rs:46`) are not available to `tool`, and adding them would turn `tool` into `coder`.
- **`agent`:** `coder` cannot replace it, for two reasons named by shipped graphs:
  - `heal-fix` needs every write mediated by `Ctx::fs` with no shell (`heal-fix.toml:2-6`).
  - `failover` needs the classified `fail` port (`agent/mod.rs:124`).
- **`recall`:** this is the weaker of the two store reads. A read through a typed tool would work at runtime, but V2 could no longer prove `memory_read` at lint, and that static proof is the reason to keep it.
- **`remember`:** same argument with `memory_write`, and consolidation's merge is a store operation that a read-compute-write sequence would make non-atomic.
- **`ledger`:** this is the weakest of the 14 on capability today. It stands on atomicity: a claim with a lease by run id (`queue.rs:50`) cannot span nodes, because a crash between them leaves a row half-claimed.

### A.3 Why not eleven

Removing `recall`, `remember` and `ledger` by making store operations typed tools gives 11 kinds. `memory_search` and `memory_put` already exist as tools (`pii-tools/src/lib.rs:89-90`). It fails on four counts:

1. **It only moves the vocabulary.** The operations are unchanged, and `tool` becomes "any Rust the daemon links", which is the same fallacy as bash with better manners.
2. **It loses the static capability proof.** `required_caps(&self)` takes no config (`node.rs:47`), so V2 cannot know which capability a given tool entry needs.
3. **It loses or overpays for the effect ledger.** `is_effect(&self)` is also per kind (`node.rs:53`). `tool` is not an effect kind today, so a replay would re-dispatch a claim. Flipping it makes every read pay the replay refusal instead.
4. **It exposes store writes to models.** `ToolSet` is what `agent` draws from (`agent/mod.rs:219`), so `queue_claim` would become callable by a model.

**The principle, which also settles op-multiplexing:** the ABI makes the kind the unit of capability and effect class. Ops may share a kind only when they share what they touch, their capability and their effect class. `remember` and `ledger` meet that. A `store` kind spanning memory and queue would not.

### A.4 The hypotheses from review 045

| Missing primitive (re-ranked) | Verdict | Evidence |
|---|---|---|
| 1. Value computation | **Confirmed, and the only real addition** | `expr.rs:6`; `boss-watch.toml:29-36` documents the workaround |
| 2. Error or handler scope | **Refined, not a primitive** | The one real gap is budget exhaustion, which cannot be routed (`state.rs:262`), so a ledger write lives outside the graph and is keyed on a graph name (`heal.rs:420`). With arithmetic the graph counts attempts and exits on a port first. |
| 3. Typed effect over a daemon resource | **Refined** | Not one primitive: three kinds, split by capability (A.3) |
| 4. Agent/model call as one primitive | **Refined** | `plan` collapses into `agent`. `coder` and `agent` do not merge (A.2). |
| 5. Durable await | **Refuted as missing** | It exists twice, as `timer` and `gate`. No graph needs a third form. |
| Bounded per-item construct | **Refuted as a primitive** | Budgeted cycle plus arithmetic is enough; shape in C.6 |

Per-kind verdicts are folded into B.2.

---

## B. DERIVATION

`transform` below means the widened form. "Disappears" lists kinds that leave that file.

### B.1 All 32 graphs

| Graph | Base kinds | Disappears | Notes |
|---|---|---|---|
| `books` | tool, route, transform, call, refuse | none | Both `$.input` shell strings go. `inventory` and `verify` become pinned calls to `docs-stale` and `docs-verify`. |
| `claude-probe` | coder, transform, refuse | none | Unchanged |
| `code-index` | timer, tool, transform, route, refuse, remember | worker, gate, memory | Rewrite required: it has no `name`/`entry` and no edges, only `depends_on`. `pii index code` becomes a literal argv with no model. Its `gate` is a check, not a human decision, so transform plus route. |
| `coder-demo` | coder, transform, refuse | none | Its `criteria` is a shell string not in the table, so it fails at run (`coder.rs:208-217`, by reading) |
| `coder-podman` | coder, transform, refuse | none | Same criteria defect |
| `coder-selffix` | coder, transform, refuse | none | Unchanged |
| `digest-now` | recall, post, transform | none | Unchanged. Becomes the callee of `discord-digest`. |
| `discord-digest` | timer, call, (callee's) | recall, post (moved) | `timer` plus a pinned call to `digest-now`, removing the copy |
| `discord-triage` | transform, post | classify | The class comes from `transform`. `route` is not needed because this graph never branches on the class (`classify.rs:108`). |
| `doc-code` | tool, route, transform, call, refuse | none | Both commands are already literal, so they become argv |
| `docs` | coder | none | Unchanged, so no pin moves |
| `fragments/podman-guard` | route, refuse | none | Unchanged |
| `intervention-report` | timer, recall, agent, transform, route, post, remember | memory, worker, gate | Rewrite required, same load defect. `gate.command` is not a key; section counting and the `TODO` check become transform plus route. Writing text needs no filesystem, so `agent` with no tools, which needs the precondition in step 5. |
| `night` | timer, call, transform, post | none | Depends on `sdd`. `in_ms = "$.input.in_ms"` appears to fail `as_u64` (`nodes.rs:346`), **unverified**: I did not trace where nodes are built. |
| `preflight` | judge, transform, refuse | none | Unchanged |
| `queue` | ledger, call, transform, route, remember | select, record | Claim, four ports, lease. The verdict's interpretation (`queue.rs:271`) becomes transform plus route, then `queue.set_status`, then `remember op=doc`. |
| `sdd` | judge, call, transform, refuse | none | Depends on `tdd` |
| `self-improve` | transform, glob/fs_read/fs_write via tool, agent, route, ledger, call | plan, enqueue | Number and slug come from transform. Parsing `## Files` and `## Exit Criteria` moves into transform; the duplicate check stays inside `queue.put`. |
| `spec` | coder, judge, call, transform, refuse | none | Unchanged |
| `tdd` | coder, judge, transform, refuse | none | Remove three one-armed joins |
| `starter/assistant` | agent, transform | none | Unchanged |
| `starter/boss-watch` | timer, tool, transform, route, ledger, gate | heal_detect | See the note on `heal_detect` in F. `cyc` can now merge values properly. |
| `starter/consolidate` | timer, remember, transform | memory | `op = "consolidate"` |
| `starter/crash` | **`debug_crash` only** | none | **The one failure at 14.** A total, capability-free set cannot panic a node by design. Move to a fixture, or count 15. |
| `starter/failover` | agent, route, transform | none | Unchanged |
| `starter/heal-fix` | judge, route, agent, transform, tool, ledger, gate | heal_record | `probe` becomes `judge expect="fail"`, `retest` becomes `judge`, `summarize` becomes argv. Needs the judge widening in step 3. |
| `starter/heal-watch` | timer, tool, transform, route, ledger, gate | heal_detect | Graph-level caps must stay free of `fs_write`, which needs step 2 |
| `starter/react` | agent, transform | none | Unchanged. `tools` includes `bash`: see B.3. |
| `starter/slow` | tool, transform | wait | `run` with argv `["sleep","30"]`, which keeps the run active. A `timer` would make the shutdown test weaker. `scripts/phase3/shutdown.sh` is the check. |
| `starter/soak` | transform | none | Unchanged |
| `starter/soak_cron` | timer, transform | none | Unchanged |
| `starter/verify-fix` | agent, judge, route, transform | tool | `test_command` becomes a spec and base. Same `bash` note. |

**Result: 31 rows are expressible and 1 is not (`starter/crash`).**

### B.2 All 23 kinds (test 3)

| Kind today | Replaced by | 045 verdict |
|---|---|---|
| `transform` | itself, widened | n/a |
| `refuse`, `timer`, `call`, `route`, `coder`, `agent`, `post`, `judge`, `tool` | themselves | `judge` item mode (`queue.rs:118`) has no shipped user, since all 8 judge nodes are spec mode. Delete it, so **no leftover mode**. |
| `gate` | itself | **Confirmed** |
| `recall` | itself | **Confirmed** |
| `select` | `ledger op=queue.claim` | **Refined:** admit the transaction, not the name |
| `record` | transform, route, `ledger op=queue.set_status`, `remember op=doc` | **Confirmed** |
| `enqueue` | tool, transform, route, `ledger op=queue.put` | **Confirmed** |
| `plan` | tool, transform, `agent`, tool | **Confirmed**, with a precondition: `tools = []` currently means all tools (`agent/mod.rs:220`) |
| `memory` | `remember op=consolidate` | **Refined:** not a graph, because the merge is a store operation; the name goes |
| `classify` | transform | **Refuted:** do not widen `route`, since the one user never branches |
| `worker` | `coder`, `agent` or `tool`, per graph | **Confirmed and sharper:** the kind requires `worker` and `op` (`worker.rs:73-82`), and neither graph sets them |
| `heal_detect` | tool, transform, route, `ledger op=heal.record_run` | new |
| `heal_record` | transform, `ledger op=heal.record_run` | new |
| `wait` | `tool run` | new |
| `debug_crash` | **nothing:** `starter/crash` needs it | new |
| `queue` (045 row) | not a kind | **Confirmed** |

Registered kinds no graph ships:

- `emit` derives to `tool fs_write`.
- `require` and `sop` are generated by the loader or SOP compiler, not written by authors.
- `spawn`, `hal`, `mqtt` and `wasm` touch their own resources and have no shipped graph. Under the "prefer deleting" constraint they are outside the set, and each is admitted when a named graph needs it.

### B.3 What restricting `tool` does not restrict (test 4)

The proof does not rest on these, but you should know they remain. A shell survives in three places, each with a named authority:

- **`judge`:** runs `/bin/sh -c` on criteria from a pinned commit.
- **`coder`:** the external agent does what it does.
- **`agent` with `bash` in `tools`:** `starter/react` and `starter/verify-fix`, chosen by a model and visible in the file.

No derivation uses any of them to fake a deterministic step. `code-index` does this today, telling a model to run a verb, and that goes away.

---

## C. THE COMPUTATION PIECE

### C.1 Choice: (b), a node kind, delivered by widening `transform`

The new keys are `let` and `out`, with no new kind name. All 43 existing `transform` nodes keep working, and no pin moves.

| Criterion | (a) expression language | **(b) node kind** | (c) typed language |
|---|---|---|---|
| Pinnability | Logic spreads into every config string, the same defect as logic in `brief` | **All logic is in named nodes under the sha256** | Good, but a reviewer must also know the language |
| Capability ladder | Evaluated inside effect kinds, so it runs holding their caps | **`required_caps` empty; `run` already ignores `_ctx` (`nodes.rs:44`)** | Same as (b), with a larger surface to audit |
| Resumability | Re-evaluated by each host | **Not an effect; the value is journaled by `NodeFinished`** | Same as (b) |
| First-attempt writing | Grammar to learn, in every field | **Same shape as `route` rules, already in the library; bad names refused at lint** | Grammar, precedence, parse errors |
| Engine impact | Changes `expr.rs`, used across kinds | **None** | A parser to own |

### C.2 Semantics

- **Shape:** `let` is an ordered array of inline tables, each binding `as` to one function call. `out` is the result, and nested `$.` is allowed there.
- **Paths:** `$.input`, `$.payload`, `$.nodes` and `$.let` go through the existing `eval`. `expr.rs` does not change. `fmt` reuses `post`'s `${...}` substitution (`post.rs:6`).
- **Total:** no recursion, no user functions, and `map`/`filter` take one level of body with `$.it`. Lists and output are size-capped.
- **Deterministic:** no clock, randomness or environment. Time comes from a `tool run` of `date`, whose journaled value replays.
- **Failure:** a type mismatch or an exceeded cap goes to the `err` port and names the `let` step. It never panics.
- **Lint:** `ports(cfg)` rejects an unknown function, wrong arity or a reference to an unbound `$.let`, reported as V0.

### C.3 The closed function table, each justified by a graph

| Functions | Needed by |
|---|---|
| `fmt`, `trim`, `lower`, `split`, `lines`, `between`, `replace`, `starts_with`, `ends_with`, `contains` | `docs-stale`, `discord-triage`, `self-improve` |
| `slug`, `to_int`, `max`, `add` | `self-improve` (next spec number, file name) |
| `case` | `discord-triage` (ordered rules) |
| `len`, `at`, `first`, `concat`, `unique`, `sort`, `join`, `map`, `flat_map`, `filter`, `in` | `docs-stale`, `self-improve`, per-item loops |
| `eq`, `ne`, `lt`, `and`, `or`, `not`, `if`, `coalesce` | `queue` (verdict), `intervention-report`, `code-index`, loops |
| `parse_json` | `heal-watch` (`verdicts.jsonl`) |
| `hash` | `heal-watch`, `heal-fix` (must equal `pii_store::hash`, since rows dedupe on it) |
| `sub` | `heal-watch` (verdict age) |

### C.4 Worked example: `pii docs stale` as a graph (test 5a)

This graph needs no per-item reads, because one `grep` and one `git diff` cover all 20 chapters (9 in `user-admin`, 11 in `arch-dev`). It uses only `transform`, `tool`, `route`, `refuse`, under `fs_read` and `proc`.

```toml
# graphs/docs-stale.toml: the chapters of one book whose stamp is behind, or "nothing-stale".
name = "docs-stale"
entry = "paths"
exit = "report"
needs = ["book", "root"]
caps = ["fs_read", "proc"]

[[node]]
id = "paths"
kind = "transform"
let = [
  { as = "src", fn = "fmt", text = '${input.root}/_docs/manuals/src/${input.book}/src' },
  { as = "main", fn = "fmt", text = '${let.src}/main.tex' },
  { as = "dir", fn = "fmt", text = '${let.src}/chapters' },
]
out = { main = "$.let.main", dir = "$.let.dir" }

[[node]]
id = "main"
kind = "tool"
tool = "fs_read"
caps = ["fs_read"]
args = { path = "$.nodes.paths.main" }

[[node]]
id = "stamp"
kind = "transform"
let = [
  { as = "hex", fn = "between", of = "$.nodes.main.content", from = 'commit \texttt{', to = "}" },
  { as = "commit", fn = "fmt", text = '${let.hex}^{commit}' },
]
out = { hex = "$.let.hex", commit = "$.let.commit" }

[[node]]
id = "has_stamp"
kind = "route"
on = "$.nodes.stamp.hex"
rules = [{ eq = "", port = "none" }, { port = "ok" }]

[[node]]
id = "head"
kind = "tool"
tool = "run"
caps = ["proc"]
args = { argv = ["git", "-C", "$.input.root", "rev-parse", "HEAD"] }

[[node]]
id = "resolved"
kind = "tool"
tool = "run"
caps = ["proc"]
args = { argv = ["git", "-C", "$.input.root", "rev-parse", "--verify", "-q", "$.nodes.stamp.commit"] }

[[node]]
id = "chapters"
kind = "tool"
tool = "glob"
caps = ["fs_read"]
args = { pattern = "*.tex", cwd = "$.nodes.paths.dir" }

# The computation Rust does today, reviewable in this file.
[[node]]
id = "decide"
kind = "transform"
let = [
  { as = "files", fn = "sort", of = "$.nodes.chapters.files" },
  { as = "behind", fn = "ne", a = "$.nodes.resolved.stdout", b = "$.nodes.head.stdout" },
  { as = "stale", fn = "if", cond = "$.let.behind", then = "$.let.files", else = [] },
  { as = "named", fn = "map", of = "$.let.stale", do = { fn = "fmt", text = '${input.book} chapters/${it}' } },
  { as = "text", fn = "join", of = "$.let.named", with = "\n" },
  { as = "empty", fn = "eq", a = "$.let.text", b = "" },
]
out = { text = "$.let.text", empty = "$.let.empty" }

[[node]]
id = "report"
kind = "transform"
let = [{ as = "line", fn = "if", cond = "$.nodes.decide.empty", then = "nothing-stale", else = "$.nodes.decide.text" }]
out = "$.let.line"

[[node]]
id = "no_stamp"
kind = "refuse"
join = "any"
reason = "the book has no commit stamp on its title page"
# Edges: paths → main → stamp → has_stamp(ok) → head → resolved → chapters → decide → report;
# has_stamp(none) and every tool or transform err → no_stamp.
```

**The derivation found a bug in the verb, and you need to decide which behaviour is the spec.**

- `is_stale` returns `true` whenever the stamp differs from HEAD (`cmd_docs.rs:124`).
- The path diff below it only runs when the stamp **equals** HEAD (`:130`), where the diff is always empty. That branch is dead.
- So any commit marks every chapter of a book stale, and `books` re-documents the whole book. The file header's second clause never decides anything.

The graph above reproduces today's behaviour. The behaviour the header describes needs three more steps: one `git diff --name-only <stamp> HEAD`, one `grep` for `\texttt{`, and a `flat_map`/`filter`/`in` over them. It is still one call each, with no per-chapter effect. I recommend the header's behaviour, since today's makes the stale check nearly meaningless.

`pii docs verify` derives the same way. Claims come from the same `grep`, paths compare against `git ls-files`, the verb list becomes literal in the file, and `replace` plus `tool fs_write` moves the stamp only after a pass. Note that `git ls-files` sees tracked files while `exists()` also sees untracked ones, so that check gets slightly stricter.

### C.5 The rule (test 5b)

**A capability must be a graph; it may be a verb only if it operates the runtime itself and so must work when no graph can run (`daemon`, `run`, `graph lint`), or if it is a thin front door that starts a graph, and it may be a kind only if it is one effect or one transaction that the base kinds cannot compose under the same capability.**

Consequence: `pii docs`, `xtask doc-drift-*` and `pii index code` are all owed as graphs, in that order of ease. Your carried finding still applies: until a verb is converted, a tested verb is better than a shell string in the run input. So converting verbs comes after removing shell strings from inputs.

### C.6 Per-item loop (no new primitive)

```
init(transform) → step(transform, join="any", loop_budget=K) → more(route)
more.item → body(tool or call) → next(transform: index+1, append) → step
more.done → finish
```

- **Precedent:** `heal-fix.toml` has this shape, and `night`'s `max_entries` covers a `call` in a loop.
- **State:** read it from `$.nodes.next`, not `$.payload`. A multi-edge node's payload stays keyed by source, and `init`'s arm stays fired (`dispatch.rs:156-171`).
- **Bound:** the Kth execution ends the run with a failure (`state.rs:262`), so K must be at least items + 1. A list that is too long fails loudly rather than truncating.

---

## D. ADAPTATION ORDER

Pins constrain the order, because editing a callee moves every host's digest. All six current pins were measured to match:

```
docs ← spec ← queue ← self-improve
docs ← sdd ← night          tdd ← sdd
docs ← books, doc-code      podman-guard ← coder-demo
```

Rewrite from the top of a chain downward where possible, since top-of-chain files pin nothing that others depend on. Where a callee must change, re-pin its whole chain in that same commit.

| # | Step | Verification | Deletions now safe |
|---|---|---|---|
| 0 | Make lint true. Remove `tdd`'s three one-armed joins and re-pin `sdd`, `night`. Add a test that lints all of `graphs/**`, with an allowlist of `code-index` and `intervention-report`. Make lint build every node, because `validate` never calls `build` today. | The test exits 0. `night_calls_sdd` still passes. Lint now reports each graph that fails at build, and the list is recorded. | none |
| 1 | Add `let`/`out` and the function table to `transform` | Unit tests per function. **Every file's sha256 is unchanged**, which proves no churn. A test asserts the code path takes no `Ctx`. | none |
| 2 | `tool`: add `run` (argv, no shell), lower `required_caps` to empty and require explicit node caps. Today V2 unions required and node caps (`validate.rs:84-85`), which would force `fs_write` onto `heal-watch`. Run `is_effect` per D.1. | A node without caps is denied at run. The `git` argv works in the daemon: `run_programs` (`run_scope.rs:116`) is **unverified**, and this step must prove it. | none |
| 3 | Widen `judge` results with a capped output tail. `heal-fix` feeds compiler output to the agent, and today results carry only rc (`judge_spec.rs:199`). Rewrite `doc-code`, `verify-fix`, `heal-fix`. | `doc_code_graph`, plus a run of `heal-fix` on a seeded break: RED, fix, GREEN, gate reached | `bash` in `tool` nodes, once `books` is done in step 4 |
| 4 | Add `docs-stale` and `docs-verify`; `books` calls them | **Differential test:** verb and graph give identical output on a fresh tree, a stale tree, and a tree with no stamp. Then `books_graph`. | `cmd_docs.rs` becomes a thin front door |
| 5 | Make `tools = []` mean none. Rewrite `self-improve`. | `self_improve_composes`. A test proves a no-tools agent cannot call `bash`. Output spec path and row are equal to the old nodes'. | `plan`, `enqueue`'s parser |
| 6 | `discord-triage`; `digest-now` and `discord-digest` | Table test: each of today's rule examples gives the same class through both. `post_e2e`. | `classify` |
| 7 | Add `ledger`. Rewrite `queue`, re-pin `self-improve`. Rewrite `heal-watch`, `boss-watch`, `heal-fix`. | `queue_calls_spec`, `queue_in_a_call`, `boss_watch`. A row is parked on refusal and untouched on an empty queue. Ledger row equal to the old one for the same input, including `hash`. | `select`, `record`, `enqueue`, `heal_detect`, `heal_record`, and the name check at `heal.rs:420` |
| 8 | Add ops to `remember`. Rewrite `consolidate`, then `code-index` and `intervention-report`. Fix `coder-demo`, `coder-podman`. | Allowlist is empty. `cron_runtime`, and `daemon/config.rs:867` still resolves `starter/consolidate`. | `memory`, `worker` as a graph kind |
| 9 | `slow` to `tool run`. Move `crash`, or admit it as the 15th. | `scripts/phase3/shutdown.sh`, `crash.sh` | `wait`, `debug_crash` from the main registry |
| 10 | Assert the registry | A test: the shipped library's kinds equal the agreed set in both registries (`registry.rs:25`, `:205`) | |

### D.1 Decision inside step 2: `tool` as an effect kind

`plan` is an effect kind and `tool` is not, so moving a write from `plan` to `tool` loses the replay guarantee. I recommend making `tool` an effect kind. The cost is that a crash during any `tool` step, including a read, then needs `pii run retry`, because `effect.unknown` refuses the replay (`effect.rs:80`). I prefer that to a silent repeated write. The alternative is a separate read kind, which would be a 15th kind.

---

## E. THE LINT

In the order the refactor needs it. Each rule refuses from the step named.

| Step | Refuses |
|---|---|
| 0 | Any file under `graphs/**` that fails lint and is not on the allowlist. The allowlist may only shrink. |
| 0 | Any node whose `build` fails. This catches `memory op="put"`, `worker` with no `worker` key, and possibly `night`'s timer. |
| 1 | Unknown function, wrong arity, unbound `$.let`, nesting deeper than one level |
| 1 | `transform` with `let` whose `err` port is unwired |
| 2 | In `tool` nodes: `bash`; a `$.` in argv0; any tool outside the five entries; missing `caps`. Grandfathered only for the 8 existing nodes, and that list may only shrink. |
| 2 | An argv hole whose value starts with `-`, refused at run, since this is option injection without a shell |
| 2 | `tool run` output that was truncated, unless the node reads the flag (see G) |
| 5 | `agent` listing `bash` unless the node names `proc` |
| 6 | A kind not in the agreed set or on the shrinking grandfather list. **This is what stops a half-migrated library shipping:** a new specialised kind fails, and a deleted kind reappearing fails. |
| 7 | An effect kind's `err` or `fail` port with no edge. V5 only warns today, and these lines are lost in the noise (`queue` has 19 warning lines). |
| 7 | Any `err` into a terminal `transform`, not only `coder`'s. V11 (`validate.rs:229`) misses `self-improve`'s `refused` and `digest-now`'s `refused_*`, which finish as successes. |
| 7 | A `call` with no pin or a stale pin, and a host that does not wire a callee's refusal. Check what `woven_pin_drift_refuses` already covers first. |

---

## F. NOT-VOCABULARY

The refactor does not fix these, even when every row in B is done.

**Named in the brief:**

| Defect | State after the refactor |
|---|---|
| Two files that do not load | Fixed by rewriting them, not by the set. The real count is five lint failures. |
| Verdicts no host wires | Remains. `select`'s `running` port has no edge in `queue.toml`, and that survives as a route arm. Only E step 7 catches it. |
| Copies (`digest-now` ≈ `discord-digest`) | Solved with `call`, which already exists. The three `coder-*` demos also remain near-copies. |
| Op-multiplexing | Reduced, not ended. `recall`, `remember` and `ledger` still have ops. A.3 gives the limit, but it is a discipline and the kind system does not enforce it. |
| Logic in `brief` strings | Remains. `transform` gives logic a better home, but nothing can lint English. `tdd.toml`'s "Do NOT write the implementation" is enforced by nothing except `judge`. |
| Untyped port values | Remains. `$.` still returns null on a typo (`expr.rs:31`), and `eq` on null just takes the fallback. Lint can check `let` names, not what a node emitted. This is the largest remaining risk. |

**Found during this derivation:**

| Defect | Evidence |
|---|---|
| `is_stale`'s second clause is dead | `cmd_docs.rs:124-132` |
| `heal_detect` ignores its `gate` setting, so `boss-watch` never runs `silent-noop-check` and `heal-watch` never runs `reachability-check`; the name is used only in the signature | `heal.rs:97`, `:243` |
| `flake` cannot fire in production: both "attempts" read the same file | `heal.rs:207-215`, `:99` |
| `coder-demo` and `coder-podman` criteria are shell strings the named table refuses | `coder.rs:208-217`, `criteria.rs` |
| `tools = []` grants every tool, including `bash` | `agent/mod.rs:220` |
| `judge` accepts `- [x]` and `enqueue` refuses it, so one spec gets two verdicts | `judge_spec.rs:48`, `enqueue.rs:55-63` |
| `gate` is declared `PURE` while reading a store | `gate.rs:27` |
| The verb list in `cmd_docs.rs` is a manual copy of `main.rs` dispatch | `cmd_docs.rs:17` |
| `debug_crash`'s comment says it is not in the lint registry; it is | `crash.rs:14`, `registry.rs:280` |
| `queue`/`heal` writes hold no capability | `queue.rs:96,327` |

---

## G. RISKS

What breaks silently while the library is half-migrated:

1. **Suspended runs meet deleted kinds.** `night.toml`'s header says a suspended run keeps its spec, and `discord-digest`, `boss-watch` and `heal-watch` suspend on timers or gates. Deleting a kind can leave a run that can never resume. **Unverified how each resume rebuilds nodes.** Drain or cancel suspended runs before each deletion in D, and add a test that resumes across it.
2. **Installed graphs drift from the repo.** The daemon reads its own graph directory (`shutdown.sh` copies files there), so a repo at step 8 can coexist with an installed `starter/consolidate` using `memory`. Cron then fails on each tick, which is loud in a log nobody watches. Check installed graphs in step 10.
3. **The two registries drift.** `registry()` and `lint_registry()` are separate lists, so a kind deleted from one lints clean and fails at run, or the reverse. Step 10's test exists for this.
4. **`transform` errors finish as success.** An unwired `err` ends that branch, and V5 only warns. A typo in a `let` could end a run reporting success, which is exactly the failure this project keeps paying for. The step 1 lint must land together with the feature.
5. **Truncated output makes a wrong decision.** Bash output is capped at 64 KB with a marker appended (`bash.rs:22-23`), and `glob`/`grep` return `truncated`. A computed stale list from cut output looks valid. `run` must report it, and graphs must route on it.
6. **Semantic drift in a rewrite.** Each deleted kind holds small behaviours nobody listed, such as `route` trimming strings (`nodes.rs:109`), `classify` lowercasing, and `plan`'s never-replace rule. That rule also becomes check-then-write, a small race it does not have today. This is why D uses differential tests rather than composition tests.
7. **Tests pass on the wrong thing.** About 50 test files reference `graphs/`, and many assert structure. `night_graph_composes.rs` is noted in `queue.rs:344-346` as passing on edges alone. A rewrite can keep the shape and lose the behaviour.
8. **Pin churn hides content.** A re-pin commit moves digests across a chain, and reviewers tend to skim those. Keep each re-pin separate from logic edits where possible. The goldens under `crates/pii-cli/tests/golden/lane/` and `graphs/COMPOSABILITY.md`, `graphs/DOCS/spec.md` also carry names or digests.
9. **Name-coupled code.** `heal.rs:420` matches the literal `starter/heal-fix`, `cmd_graph_installed.rs:375` maps kind names, and `xtask/src/promote_build.rs` and `rss_gate.rs` depend on `starter/soak`.
10. **Rewriting removes a safety net that looks unused.** `is_stale`'s behaviour makes `books` document everything. Fixing it means fewer docs runs, which may expose chapters that were only correct because they were constantly rewritten.

---

## Decisions for you

1. **`starter/crash`:** move it to a fixture for 14 kinds (recommended), or keep it in `graphs/` and accept 15.
2. **`pii docs stale`:** implement the behaviour the header describes (recommended), or preserve today's "any commit makes every chapter stale".
3. **`ledger` capability:** add one bit in `pii-abi` (recommended), or leave queue writes open as they are today.
4. **`tool` as an effect kind:** yes with the retry cost (recommended), or a separate read kind as a 15th.

## Verification of this analysis

- **Measured:** kind counts, all lint exit codes, pin digests, chapter counts, and the file:line citations.
- **Needs a run before relying on it:** the C.4 graph, argv scope for `git` (`run_scope.rs:116`), resume across a deleted kind, `night`'s `in_ms`, and whether V9 blocks `run start`.
- **Commands:** `target/debug/pii graph lint <file>` for each file, then `cargo test -p pii-server --test night_calls_sdd --test books_graph --test queue_calls_spec` once steps begin.
