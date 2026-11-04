> Derived by the reviewer (read-only, `--permission-mode plan`) in answer to the brief beside this
> file. Committed by the orchestrator unchanged. Nothing here is ratified: the operator rules on it.

# 047 — The minimal set of graphs pii needs

**Context.** Answers `_docs/06-research/047-minimal-graph-set-brief.md`. Read-only: no file in the tree was changed.
Measured 2026-11-04 with `target/debug/pii` (0.2.0), `sha256sum`, and a copy of `~/.local/state/pii/store.redb`.

**The answer: 11 system graphs.** 9 exist and load, 1 exists and does not load (`code-index`), 1 is new
(`method-judge`). The set closes on paper, but 6 of its 13 flows do not work today, and 3 primitives block the
unattended loop. Rows 384 and 385 are still `pending` and each encodes something this derivation changes (see
"What changes in the pending specs"), so amend them before anyone implements them.

Evidence about what has actually run, from this host's store (`Started` records; superseded pages may inflate them, so
these are upper bounds):

```
discord-digest 1852 · default 26 · m0-daemon-starts-a-container 12 · queue 8 · coder-demo 3 · every other graph 0
```

- `night`, `sdd`, `tdd`, `self-improve`, `books`, `doc-code`, `digest-now`, `preflight` and `code-index` have 0 here.
- `spec` also has 0 here, but `spec.toml:12,18` cites real runs (`r000000000130`, `r000000000260/261`) from another
  store. I did not inspect stores other than this host's.
- The only night ever run was driven by Hermes scripts, not by the graph (`_docs/05-operations/night-loop.md:3-9`).
- `night`, `sdd`, `tdd`, `self-improve` and `books` are not installed on this host at all (`pii graph list`).
- Installed `spec` and `queue` are older than the repo: installed digests `30580132…` and `1fafe548…`, repo `771d098b…`
  and `63c2678c…`.

---

## A. Flows

An operator must be able to ask a host to:

1. **F1** implement a named spec and get back exactly one verdict: done, unverified or refused.
2. **F2** implement it test-first, refusing a test that passed before the code existed or stayed red after it.
3. **F3** implement it when its criteria are not new tests (config, docs, `grep`), proved by the criteria alone.
4. **F4** document verified work in a separate session, refusing a docs step that breaks the criteria.
5. **F5** work the next queue row and write the verdict onto that row, never leaving it claimed.
6. **F6** turn a stated finding into one admitted spec and one queue row, and run it.
7. **F7** find the next gap by itself when nothing is queued.
8. **F8** run unattended from a clock: drain, fill when empty, review, refill, and report every outcome including refusals.
9. **F9** have an independent model review what landed, once all rows are closed, and file a report with the next findings.
10. **F10** bring whatever an inventory command calls stale up to date, proving every path and verb named is real.
11. **F11** say on request what the index knows.
12. **F12** index a tree into memory, so recall can answer.
13. **F13** prove the host can judge criteria before anything is spent.

F1–F10 are the documented loop (detect, spec, implement, judge, review, report). F11–F12 are the "index" half: without
them pii has no graph that writes to or reads from its own index. F13 exists because a real failure class was caught
there (`preflight.toml:1-3`, spec 301).

## A2. System vs user

**System (11):** `spec · method-judge · sdd · docs · queue · self-improve · night · books · digest-now · code-index · preflight`.

**"Works on a fresh host"** means: lints, closes, and any run reaches a *named* verdict with no operator graph or file
present. Secrets, a channel and a model are configuration, not content. Blocked today by P4 and P5 in section G.

**User:** everything the operator writes or copies, in `~/.local/pii/graphs/`, beside the config the installer already
places in `~/.local/pii/`. An upgrade never writes there.

### What enforces the direction: three layers, one of them structural

| layer | mechanism | what the operator sees |
|---|---|---|
| **Search path (the enforcement)** | `load_host_with_graphs` already takes the dirs to search (`crates/pii-graph/src/fragment/weave.rs:112`). A host loaded from the system dir gets `[system]`; a user host gets `[system, user]`. A system graph cannot resolve a user target at all, whatever its text says. | `graph.call.not_found: spec calls 'my-method' — searched /…/system (11); a system graph cannot call a user graph` |
| **No shadowing** | `find_graph` is first-match today (`crates/pii-server/src/graph_paths.rs:46`). A user file named like a system graph is **refused at load**, not preferred: shadowing `docs` would make every system graph call user code, defeating layer 1 by name. | `graph.name.reserved: ~/.local/pii/graphs/docs.toml — 'docs' is a system graph; copy it under another name` |
| **Install-time and CI check** | Spec 384 R3/R4 extended: every manifest graph lints, every `call` target is in the manifest, every pin matches. Fails before any file moves. | `install refused: night calls 'x', which is not in graphs/minimal.txt` |

`pii graph edit` on a system graph is refused and suggests copying it. Today it writes in place
(`crates/pii-cli/src/cmd_graph_installed/edit.rs:175`); this host's five `queue.toml.prev-*` are that habit. `pii graph
list` gains a `system|user` column in place of today's `installed-only`.

### Pins across an upgrade: keep the digest, retain the old revision, re-pin by hand

The installer keeps every replaced system revision at `system/.by-sha256/<digest>.toml`. A `call` whose pin differs from
the current file resolves there. The pin's check stays; today's pin-mismatch refusal
(`crates/pii-graph/src/fragment/call.rs:202`) now applies only when no retained revision exists.

| rejected | why |
|---|---|
| refuse the upgrade | one forgotten operator file would block every system fix |
| re-resolve the pin with a report | breaks the promise `COMPOSABILITY.md:57` exists to make: a run nobody reviewed, with a report afterwards |
| name + version | a label does not bind content, and the re-pin stops being a reviewable hash in a diff |

What the operator sees on every upgrade (`--check` prints it and changes nothing). `pii graph diff` and `repin` are new verbs:

```
pii 0.2.0 → 0.3.0   system: 11 installed, 11/11 lint, closure ok
user graphs: 3
  my-night   call queue  63c2678c → 9ab1…  BEHIND  still runs 63c2678c
             review: pii graph diff queue 63c2678c 9ab1…   accept: pii graph repin my-night queue
  my-docs    call docs   b7f8eacd          OK
  old-probe  kind 'classify' is gone       BROKEN  refuses at start · armed cron: none
```

The upgrade still completes on BROKEN, and the line is loud when a cron is armed on that graph. The cost: BEHIND means
running without the system fix, and the report says so rather than hiding it.

**First upgrade only:** anything in `~/.local/state/pii/graphs` not in the manifest is **moved unchanged** to the user
dir and listed. That covers `discord-digest` (armed, and the one graph with a real production history) and
`m0-daemon-starts-a-container`. Nothing is deleted.

### The samples

| file | decision |
|---|---|
| `coder-demo`, `coder-podman`, `claude-probe`, `discord-triage` | examples to copy into the user dir; not installed |
| `coder-selffix`, `discord-digest` | obsolete in the repo (section E); `discord-digest` survives on hosts through the move above |
| `digest-now` | **system** (F11) |

**The installer must change with this.** It aborts if `discord-triage` or `discord-digest` is missing, and generates a
config naming both (`deploy/install-pii.sh:96,111,164-166`). When a Discord token is supplied it should copy
`discord-triage` to the user dir and stop writing the digest cron.

---

## B. Flow → graph

| flow | owner | composition | duplicate found → decision |
|---|---|---|---|
| F1 | `spec` | route → method → `docs` → reverify | `sdd`'s tail repeats it (`sdd.toml:49-71`) → the tail leaves `sdd` |
| F2 | `sdd` | — | `tdd` + `sdd` → **`sdd` survives, `tdd` is merged in** |
| F3 | `method-judge` | — | inline in `spec.toml` today → extracted |
| F4 | `docs` | reverify is `spec`'s | none |
| F5 | `queue` | → `spec` | none (spec 364 removed the old one) |
| F6 | `self-improve` | → `queue` | none |
| F7 | `self-improve` | new `detect` tool node, `pii incidents scan --new` | none |
| F8 | `night` | → `queue`, `self-improve`, `docs` | none |
| F9 | `night` | `call docs` with the review brief | none; no new graph needed |
| F10 | `books` | → `docs` | `doc-code` → **`books` survives** |
| F11 | `digest-now` | — | `discord-digest` → **`digest-now` survives** |
| F12 | `code-index` | — | none |
| F13 | `preflight` | — | none |

### Decision 1: `sdd` vs `spec`. Accept 385's principle, change its shape

**Accepted:** `sdd` is not a duplicate of `spec`. They are one driver with two methods baked in, so the driver routes
between static pinned calls.

**Changed, on the removal test:**

1. **Remove `docs` and `reverify` from `sdd`.** Otherwise a driver that calls `sdd` and then `docs` documents twice.
   385's diagram implies this, but its R2 and exit criteria never require it.
2. **Remove `sdd`'s `verify`.** It repeats `tdd`'s `green` with nothing between them: same spec, base and workdir.
3. **That leaves `sdd = contract + call tdd`,** a one-node wrapper with no flow of its own. Both files fail "remove any
   member and a flow loses its owner", so they merge: `sdd` = `contract → write_test → red → implement → green`.

**Which name survives:** `sdd`. It matches 385's exit criterion and the operator's correction. That correction objected
to losing the *capability*, and RED-before-GREEN is fully kept. This merge is a change the operator has not seen, so it
is the first item to check. `tdd` is arguably the more accurate name; the graph count is the same either way.

**`method-judge` could be inlined** in `spec` for one graph fewer. Rejected: that is the arrangement that made `sdd`
look like a duplicate, and it would make a user driver copy a coder node rather than call it.

**A third methodology** cannot be reached through the system driver: system cannot call user. The operator writes a
user driver (route → own method → `docs` → reverify) and a user copy of the 4-node `queue`. That is the honest answer to
"what happens if using other methodology"; 385's "a matter of writing one" understates it.

### Decision 2: the night. The document is right and the graph is wrong

- `night.toml` is one pass: no back edge exists, so `max_entries = 12` (`:32`) bounds nothing.
- Its `review` is a pass-through (`:44-48`, `expr = "$.payload"`), though `graphs/README.md:64` draws it as a judge.
- It drops `sdd`'s refusals (spec 381): a refused night reports nothing.
- Calling `queue` instead also fixes the reporting. `record` turns every verdict and an empty queue into its `ok` port
  (`crates/pii-server/src/node_kinds/nodes/queue.rs:254-309`), so the night can route on `$.payload.status`.

```
tick → drain (call queue) → route on status
  done | parked  → drain                      (bounded by max_entries; spec 306, tests/call_max_entries.rs)
  blocked        → report
  empty          → open rows? ── yes → report, naming them; no review   (night-loop.md:25-29)
                                 no  → fill (call self-improve) → route
                                         ran a row    → drain
                                         nothing-to-do → review (call docs) → findings? yes → fill · no → report
```

385 R4 ("`self-improve` and `queue`, in that order") contradicts its own Q2 (fill only when empty). The order is drain
first. `self-improve` already contains a `queue`, so the fill path needs no second drain call.

The cron stays opt-in: a fresh host must not start spending unattended.

---

## C. The set

| graph | purpose | entry | exit | calls | caps | breaks if removed |
|---|---|---|---|---|---|---|
| `docs` | one coder, on the host's brief | `docs` | `docs` | — | spawn, proc (see risk 1) | F4, F9, F10 |
| `sdd` | prove RED, implement, prove GREEN | `contract` | `green` | — | spawn, proc, fs_read, fs_write | F2 |
| `method-judge` **(new)** | implement, then judge | `code` | `verify` | — | spawn, proc, fs_read, fs_write | F3 |
| `spec` | driver: choose a method, document, re-judge | `method` | `done` | `sdd`, `method-judge`, `docs` | spawn, proc, fs_read, fs_write | F1 |
| `queue` | one row, one recorded verdict | `select` | `record` | `spec` | spawn, proc, fs_read, fs_write | F5 |
| `self-improve` | finding or detected gap → spec → row → run | `case` | `report` | `queue` | spawn, proc, fs_read, fs_write | F6, F7 |
| `night` | the unattended loop | `tick` | `report` | `queue`, `self-improve`, `docs` | spawn, proc, post, fs_read, fs_write | F8, F9 |
| `books` | stale inventory → docs → proof of claims | `inventory` | `done` | `docs` | fs_read, fs_write, proc, spawn | F10 |
| `digest-now` | what the index knows, on request | `devices` | add `digest` | — | memory_read, post | F11 |
| `code-index` | index a tree, report the counts | add on the 380 fix | add | — | proc | F12 |
| `preflight` | can this host judge, for no spend | `verify` | add `done` | — | proc | F13; also `pii lane` (`crates/pii-cli/src/cmd_lane/`) |

Kinds the set uses: `coder judge route tool call transform refuse timer post select record plan enqueue recall`. If row
383 or brief 046 retires any of them, these graphs move with it.

## D. Closure

Nine edges, every target in the set, no call to a user graph:

```
night → queue · night → self-improve · night → docs
self-improve → queue · queue → spec · books → docs
spec → sdd · spec → method-judge · spec → docs
```

| | state |
|---|---|
| Today, repo-wide | **8/8 `call` pins match the files on disk**, and `coder-demo`'s fragment pin matches |
| Edges that exist today and survive | `queue→spec 771d098b` · `self-improve→queue 63c2678c` · `spec→docs b7f8eacd` · `books→docs b7f8eacd` |
| After the changes | Digests cannot be stated until the files exist. Re-pin leaves first: `docs` → `sdd`, `method-judge` → `spec` → `queue` → `self-improve` → `night`, and `books`. Spec 384 R4 makes a mismatch refuse the install. |

## E. The 32 files

| class | n | files and reason |
|---|---|---|
| **environment** | 9 | `books docs digest-now night preflight queue sdd self-improve spec` |
| **sample** | 16 | `coder-demo` (needs `coder-selffix`'s live model values; its `deepseek-v4-flash` is dead per `README.md:96`) · `coder-podman` · `claude-probe` (absolute path, `:13`) · `discord-triage` (echoes a class; no flow needs it) · `fragments/podman-guard` (a fragment; the installer never copies `fragments/`, so `coder-demo` cannot load on a fresh host) · 11 `starter/*`: `assistant boss-watch consolidate crash failover heal-fix heal-watch react slow soak verify-fix` |
| **obsolete** | 5 | `tdd` (merged into `sdd`) · `doc-code` (`books` with `inventory` and `verify` inputs; the empty-output check becomes the `nothing-stale` sentinel in the command) · `discord-digest` (the scheduled copy; decided asked-for, not scheduled, `digest-now.toml:5`; 15 references in `crates`/`deploy`/`scripts` to migrate) · `coder-selffix` (`coder-demo` with other values; one list in `shipped_coder_graphs_fail_on_err.rs` plus `scripts/deploy-lnx-test.sh`) · `starter/soak_cron` (0 references) |
| **broken** | 2 | `code-index` → fix under 380, joins the environment; drop its `0 3 * * *` timer and call `pii index code`, whose absence was the file's reason not to ship (`:7-8`) · `intervention-report` → fix under 380, then sample; F8's report and `docs` own its flow |

Total 32. Three more do not load *cleanly*: `tdd`, `sdd` and `night` lint `rc=3` (V9, spec 381). The merge and the new
`night` remove that.

"Delete" means after the tests that name them migrate. On hosts, the first-upgrade move applies.

## F. The rule for a new graph

A graph joins the environment only if you can write one sentence of behaviour an operator would ask for, with no graph
name in it, that no graph in the environment owns and that no composition of them can deliver; and it lints with no V9;
every `call` target is already in the environment and pinned; it holds no absolute path, no credential and no model id;
a run on a fresh host reaches a named verdict; a test would fail if the graph were removed; and removing it would leave
that sentence without an owner. Being useful, tested or small is not enough. If a second graph would serve the same
sentence, the old one goes `obsolete` in the same commit. Anything failing a condition goes in samples, or in the user
dir if it is specific to one operator or one repository.

## G. What is blocked

**Capabilities that cannot be a graph today:**

| # | capability | the block | primitive needed |
|---|---|---|---|
| **P1** | F6 and F8's fill: judging a spec planned in the same run | The judge reads the spec with `git show <base>:<spec>` (`judge_spec.rs:174-188`), but `plan` writes an uncommitted file (`plan.rs:1-6`), so it is not at `base` and the verdict is `failed`. **Read from the code, not run:** `self_improve_composes.rs` only loads the graph. | `enqueue` stores `git hash-object -w <spec>` and the judge reads that blob. Still immutable to the coder, and needs no commit, so the commit stays human. |
| **P2** | F8 draining more than one row | Workers do not commit, so row N+1 runs on row N's diff. A parked row's leftovers already turned the gate red for every row behind it (`night-loop.md:104`). Hermes's `clear-stuck-tree.sh` (outside this repo, in `~/.hermes/skills/autonomous-ai-agents/night-loop/scripts/`) covers this today. | One worktree per row, plus a trimmed stdout value: `is_commit_id` wants exactly 40 hex and `transform` cannot compute (brief 046). |
| **P3** | Retrying a parked row (`state.py repair`) | `pii queue` offers only `add`, `ls`, `claim` | A requeue verb reachable from `tool`, or a kind. |
| **P4** | "Report" on a host with no channel | `post` has no `err` port and `channel` is a literal (`post.rs:11,207`), so the last node fails the run after the work is recorded. | An always-present local channel, such as the journal. |
| **P5** | A system graph naming no model | `queue.toml` and `self-improve.toml` hard-code `pi`, `deepseek-v4.1-flash:cloud`, `glm-5.3`. | Not new: land spec 362 (`@workhorse`, `@planner`), now `needs-review`. |

**Test 5, every capability named or found (19):**

| disposition | n | capabilities |
|---|---|---|
| in the set | 8 | stale inventory (`books.inventory`; `$.input.dry_run` adds a read-only answer) · queue fill (`enqueue`) · digest · drain (`scripts/autonomy/drain.sh`) · morning report (`night.report`) · incident scan (`detect`) · index code (`pii index code`) · host check (`preflight`) |
| composable with `tool`, no flow requires them | 6 | `pii index docs` · `pii learn *` · `pii queue ls` · `pii docs verify` · the gate (`quality-gate.sh`) · `pii memory *` reads |
| blocked | 3 | parked retry (P3) · stuck-tree mover (P2) · `fix-row.sh` (only for hand-written rows; `enqueue`'s validators refuse a bad planned row, so it stays a Hermes script) |
| not graphs by definition | 2 groups | the control plane that starts graphs (`daemon`, `run *`, `graph *`, `install-pii.sh`) · `pii -p` and the built-in `default` graph (`engine/registry.rs:293`), outside the 32 and not examined |

No per-item iteration primitive is needed for this set: `books` hands the coder a list, and `night` fills one row per cycle.

---

## Acceptance tests

| # | test | the set as derived | the tree today |
|---|---|---|---|
| 1 | No gap | **PASS** 13/13 | **FAIL** 7/13 have a loadable owner doing the behaviour. Missing: F2 (lint rc=3), F6 (P1), F7 (no detect), F8, F9, F12 (does not load) |
| 2 | No duplicate | **PASS** 13/13 single owner; 4 displaced by name: `tdd`, `doc-code`, `discord-digest`, `coder-selffix` | **FAIL** 3 pairs serving one flow (`sdd`/`spec` tail, `books`/`doc-code`, the two digests) |
| 3 | No passengers | **PASS** 11/11 | **FAIL** by the same test, of the 19 top-level files, 6 own no flow |
| 4 | Closure | **PASS** 9/9 targets in the set; pins computed at implementation, enforced by 384 R4 | **PASS** 8/8 pins match. Installed host: not closed against the repo (see the evidence above) |
| 5 | Everything is a graph | **PASS** 19/19 dispositioned: 8 in set, 6 composable, 3 blocked, 2 groups excluded by definition | same table |
| 6 | Populations separate | **PASS** 0/9 edges from system to user; upgrade path stated | **FAIL** one directory, one search path, shadowing possible, no user population exists |

**Three derived-set PASSes depend on work not yet done:** F6–F8 need P1 and P2, and the fresh-host claim needs P4 and P5.
Those are PASSes on the design, not on anything that has run. No flow above F5 has a production run on this host.

## What changes in the pending specs

| row | change |
|---|---|
| **385** | Merge `tdd` into `sdd`; require `sdd` to lose `docs`, `reverify` and `verify` in its criteria; reverse R4's order; add `open rows` and the back edge. |
| **384** | The manifest is the 11 here. Replace its closure diagram (`night → sdd`, `doc-code`). Add the search-path split, the no-shadow refusal and revision retention. Change the installer's hard-coded graph names. |
| **381** | R1 and R3 are absorbed by the merge and the new `night`; R2 (`select.running`) stands. |
| **380** | Stands. `code-index` joins the environment and `intervention-report` becomes a sample. |
| **new** | P1, P2, P3, P4. P1 first: it is what makes the loop close. |

## Risks and open items

1. **`docs` may not be able to write, or its caps may be misdescribed. Unresolved.** It holds `spawn, proc`
   (`docs.toml:8,13`), and `README.md:24-26` says such a coder is refused writes. But the `coder` code requires only
   `SPAWN | PROC` (`coder.rs:378`), a search found no `fs_write`-to-tool mapping, and `README.md:101` blames approval mode
   for the same symptom. This store holds no `docs` node result. Settle it with one live `docs` run before relying on F4, F9
   or F10. If caps must change, every pin moves.
2. **`contract` may refuse a retried row.** It requires *all* criteria to fail (`sdd.toml:20`), so a parked row retried
   with partial work is refused on the strict path, which conflicts with spec 368. Not run.
3. **Review runs in a writing coder.** Reusing `docs` for F9 adds no graph, but nothing yet proves a review changed only
   its report. Add a `tool` check of `git diff --name-only` in `night`, or accept it.
4. **`night`'s wall.** `max_wall_ms = 7200000` is 2 h, and one queue pass may take up to 6 000 000 ms. A real night needs
   a much larger bound.

## Verification (when implemented)

- `pii graph lint` returns 0 for all 11; `minimal_graph_set` compares `pii graph list` after a scratch install against the manifest.
- Negative tests that would fail if the feature broke: a system graph naming a user target is refused; a user `docs.toml`
  is refused; a user pin to a replaced revision still runs the old bytes, then `repin` moves it.
- One real end-to-end `night` on a throwaway repo: one row done, one parked, one planned through P1, and a report naming all three.
