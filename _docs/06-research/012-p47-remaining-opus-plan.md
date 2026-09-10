# P4.7 remaining — analysis and spec plan

## 0. Premise corrections (read these first; three of the brief's assumptions are wrong)

**(a) Slice E is roughly 70% already built — by spec 08, not spec 27.** `graphs/starter/heal-fix.toml` is a complete localize→patch→validate→escalate loop: `probe` (bash, runs `$.input.test_command` in `$.input.worktree`) → `route_probe` on exit code → `fix` (agent, `tools = ["fs_read","fs_write","edit","multi_edit"]`, no bash) → `retest` → `route_retest` → `summarize` (`git diff`) → `escalate` (gate, `policy = "heal-escalation"`) → `done`, with a `retry` back-edge and `loop_budget = 2`. Worktree provisioning, disk ceiling and lifecycle exist: `xtask/src/heal_seed.rs:28` (`_tmp/heal-worktrees`), `:72` (≥50 worktrees → refuse), `:77` (>2 GiB → refuse), `xtask/src/heal_prune.rs:35-55` (`mv` to trash, never `rm -rf`). Write confinement is real and enforced at the capability layer: `crates/pii-server/src/runs.rs:261-283` (`validate_scope_root`), `:232-255` (`run_caps` — fs_write scope = worktree only, fs_read = worktree + workspace), `crates/pii-core/src/fscap.rs:97-151` (canonicalise-then-check). The only exit from the sandbox is `xtask heal-collect <sig>` (`xtask/src/heal_collect.rs:1-3`), which writes `_tmp/heal-reviews/<sig>.patch` and applies nothing.

What Slice E actually still lacks is narrow: heal-fix is **unreachable from boss-watch**, has **no `route_step`**, was only ever driven by a scripted model (`_tmp/phase3/slice1-e2e.log`), and has **no anti-reward-hacking guard**. Plan against that, not against a greenfield slice.

**(b) The boss loop cannot tick. This is the blocker for everything.** `TimerNode::run` (`crates/pii-graph/src/nodes.rs:292-311`) has no code path returning `Outcome::Next` — it unconditionally returns `Outcome::Suspend { reason: Timer, key }`. `pii_graph::run::resume` (`crates/pii-graph/src/run.rs:124-160`) replays the journal, finds no `NodeFinished` for `tick`, re-dispatches it, and it suspends again with a *new* `fire_at_ms`. `boss-watch.toml`'s `entry = "tick"` therefore never reaches `check`. `pii-evolve`'s only action — `run.start` for `starter/boss-watch` (`crates/pii-evolve/src/driver.rs:108-132`) — starts a run that suspends immediately and can never advance. `heal-watch.toml` has the identical shape. This is documented honestly in `crates/pii-server/tests/boss_watch.rs:7-28`, but it means **the shipped boss detects nothing**, and `xtask silent-noop-check` cannot see it: `xtask/src/silent_noop.rs:139` scans methods, events, config keys and graph-node keys — not node control flow. Nothing else in P4.7 is worth building until this is fixed.

Secondary: nothing schedules boss-watch at all. There is no `[[cron]]` entry for it anywhere in the repo or in `/root/.pii/pii.toml`, and `CronSection` (`crates/pii-server/src/daemon/config.rs:167-178`) is unused by any deployed config.

**(c) "Pluggable" (008 §3.5) is only half true at HEAD.** `crates/pii-evolve/Cargo.toml` has **no `[features]` block**; the root `Cargo.toml` lists `crates/pii-evolve` as an unconditional member with no `default-members`, so `cargo build --workspace` still compiles it. What *is* true — and is the load-bearing half — is that no crate depends on `pii-evolve`, so the `pii` binary links none of it. 008 §3.5 asked specifically for `cargo build --features pii-evolve`. `_docs/02-guides/feature-gating.md` still lists `evolve` as "upcoming." Decide whether structural separation satisfies the requirement (see D7).

**(d) Slice C (context management) is unbuilt, not merely un-listed.** Spec 25 deleted `context.rs`; `crates/pii-server/src/compaction.rs:69` still counts a turn as a `Role::User` message so `compact` is a no-op on any agent transcript (011 A17d, unfixed). `StateDoc` (`crates/pii-evolve/src/state.rs:20-35`) is written by the *launcher* before/after the run, not by a node inside it — the `state_write` node kind proposed in 011 was never built (`crates/pii-server/src/registry.rs:23-55` registers `agent`, `tool`, `gate`, `heal_detect`, `memory`, `debug_crash`, `worker` and nothing else). The user's "it should manage its own context" ask is not met.

---

## 1. Spec plan (30–36)

Convention: `specs/NN-<kebab-slug>.md`, sections `Verified Premises → Overview → Requirements (Functional/Non-Functional) → Architecture → TDD Contract → Exit Criteria → Guardrails → Files to Modify`. Exit criteria as `cargo test -p <crate> <prefix>_ 2>&1 | grep -qE "test result: ok\. [1-9]"`.

### Spec 30 — `timer` nodes resume (deterministic, no model) — **BLOCKER**
*Serves*: 008 §3 step 1; unblocks 27's shipped graph and `heal-watch`. Fixes the defect recorded at `crates/pii-server/tests/boss_watch.rs:7-28`.

- **R1** A suspended `timer` node whose wheel entry has fired resumes to `Outcome::Next { port: "fired" }` instead of re-suspending.
- **R2** `heal-watch` is fixed by the same change (no per-graph special case).
- **R3** The re-arm is once-per-fire: replaying a completed timer does not re-dispatch it.
- **R4** `pii graph trace <run_id>` shows `NodeStarted`/`NodeFinished` for the timer node.
- **R5** A `[[cron]]` entry for `starter/boss-watch` in the starter config, wired through `CronSection` (`config.rs:167-178`), so periodicity comes from the scheduler rather than from an external `pii-evolve` invocation.

**TDD contract (acceptance = T1).**
| test | given | expects |
|---|---|---|
| `timer_resumes_to_fired_port` | `timer(in_ms=1) → transform` graph; run, then resume via `runs::spawn_resume` after the fire time | terminal is `Done`, and the journal contains exactly one `NodeFinished{node:"tick", port:"fired"}` — **not** a second `Suspended` |
| `boss_watch_reaches_detect_after_resume` | the **real** `graphs/starter/boss-watch.toml` via `Engine::load_spec`, resumed once | journal contains `NodeStarted{node:"check"}` |
| `timer_does_not_refire_on_replay` | resume a run whose timer already finished | timer dispatch count stays 1 (assert the count, not the terminal) |

T1 must fail if the fix is reverted — today it produces a second `Terminal::Suspended`.

**Design note for the implementer**: `GraphNode` has only `fn run` (`crates/pii-graph/src/node.rs:10-16`); there is no resume hook. Two shapes — see D1.

**Guardrails**: no new node kind; `SuspendReason::Timer` and the `ResumeKey` format (`nodes.rs:300-305`) stay byte-identical so the `TIMER_WHEEL` index (`crates/pii-store/src/timer.rs`) keeps working; don't touch `gate` suspension (it already round-trips correctly via `gate.rs:83-87`).

---

### Spec 31 — protected paths: the boss cannot patch its own oracle (deterministic, no model)
*Serves*: 008 §6 ("do not let the boss edit `specs/`"), 011 A17g — **verified still open at HEAD**: no refusal for `xtask/`, `quality-gate.sh`, `graphs/`, `specs/` or a `#[test]` exists anywhere in `crates/pii-tools/src/` or `crates/pii-server/src/`.

This is the single highest-risk gap. A worktree is a full checkout, so `<worktree>/xtask/`, `<worktree>/quality-gate.sh`, `<worktree>/specs/` and every `#[test]` are **inside** the heal agent's fs_write scope (`runs.rs:232-255`). The cheapest way for any model to make `cargo test` green is to delete the assertion. Must land before spec 33.

- **R1** A protected-path deny list applied at `FsCap::write` (`crates/pii-core/src/fscap.rs:97-151`), expressed through `PathScope`'s existing deny-wins semantics (`crates/pii-core/src/cap.rs:52-57`) so it is a capability, not a policy an agent can talk its way past. Default set, relative to the run's `scope_root`: `specs/`, `xtask/`, `quality-gate.sh`, `graphs/`, `clippy.toml`, `rust-toolchain.toml`, `.github/`.
- **R2** `xtask heal-collect` refuses a patch that touches a protected path **or** whose diff net-removes a `#[test]`, `assert!`/`assert_eq!` line, or adds `#[ignore]`. Exit 1 with the offending hunk.
- **R3** The refusal is recorded: the `HealAttempt` moves to `HealState::Escalated` with the reason in `detail` (`crates/pii-store/src/heal.rs:14-28`, `:225-259`).
- **R4** `silent-noop-check` (`xtask/src/silent_noop.rs:139`) gains a class: a protected-path entry with no enforcement site fails the gate.

**TDD contract (acceptance = T2).**
| test | given | expects |
|---|---|---|
| `fs_write_denies_protected_path_in_scope` | ctx scoped to a temp worktree; write `<wt>/specs/foo.md` | `PiiError::Denied`; the file does not exist on disk afterwards |
| `heal_collect_refuses_test_deletion` | worktree diff that removes one `assert_eq!` line | exit 1, stderr names the file and the removed assertion; no `.patch` written |
| `fs_write_still_allows_source` | write `<wt>/crates/pii-core/src/x.rs` | Ok |

T2 is the acceptance test: a plausible implementation checks only the path list and lets a test deletion through under `crates/`.

---

### Spec 32 — detect hands off to heal: a `spawn` node + worktree provisioning (deterministic, no model)
*Serves*: 008 §3 steps 2→4; closes the gap between `boss-watch` and `heal-fix`.

There is **no `subgraph`/`spawn`/`map` node kind** (searched: `crates/pii-graph/src/node.rs:43-55` plus `crates/pii-server/src/registry.rs:23-55` — 12 kinds total, none of them chain runs). Today `boss-watch`'s `escalate` gate is terminal in both directions. Something must turn an approved detection into a scoped `heal-fix` run.

- **R1** A `spawn` node kind (registered in `pii-server`, not `pii-graph` — it needs the `Engine`): given `graph` + an input expression, it calls the same path as `run.start` (`crates/pii-server/src/dispatch.rs:153-172`), returns the child `run_id` on `ok`, and journals a `RunSpawned`-style link so `pii graph trace` shows the parent→child edge.
- **R2** A `worktree` provisioning step reachable from a graph: promote `heal_seed`'s git logic (`xtask/src/heal_seed.rs:81-101`) into a callable form and expose it as a `tool` (not an agent capability), returning `{worktree, branch, base_commit}` — the exact fields `HealAttempt` already carries (`crates/pii-store/src/heal.rs:86-88`).
- **R3** The child run's `scope_root` is the worktree; `validate_scope_root` (`runs.rs:261-283`) must accept it, which requires worktrees to stay under the workspace (`_tmp/heal-worktrees`) — do **not** relocate them to `/root/.pii-dev/`.
- **R4** `boss-watch.toml` gains `escalate --allow→ provision → spawn(heal-fix) → done`; `deny` still terminates. Budget for the child run is bounded by `[limits]` (all four dims now thread through `Budget::new`, `runs.rs:56-61`, `:169`).
- **R5** Rate limit: refuse to spawn a heal for a signature whose `attempt_count` (`heal.rs`) is ≥ 2 — route to a second escalation gate instead (008 §6).

**TDD contract (acceptance = T1 and T4).**
| test | given | expects |
|---|---|---|
| `spawn_node_starts_child_run` | graph with `spawn(graph="starter/verify-fix")` | a second run header exists in the store, `status = Running`, and its `input` carries the parent's payload |
| `spawn_child_scope_root_is_worktree` | spawn with `scope_root = <wt>` | child ctx denies `fs_write` to `<workspace>/crates/...` and allows `<wt>/crates/...` |
| `provision_returns_branch_and_base_commit` | provision against a temp clone | `{worktree, branch, base_commit}` all non-empty; `git worktree list` shows the new entry |
| `spawn_refused_after_two_attempts` | `HealAttempt{attempt_count: 2}` for the signature | no child run is created; routes to the escalation port |

T4 is the one a plausible implementation gets wrong — it will spawn and *then* check.

---

### Spec 33 — Slice E live: heal-fix runs on a routed model, end to end
*Serves*: 008 §5 E, §7.3 (first target = a deliberately-broken unit test). **First spec that needs a model node.**

- **R1** `heal-fix.toml`'s `fix` agent declares `route_step = "patch"`; a new `localize` agent (or the existing `probe` output feeding `fix`) declares `route_step = "localize"`. Resolution goes through `resolve_model` (`crates/pii-server/src/agent.rs:77-96`), which is loud (`NotFound`) on an unassigned step — `KNOWN_STEPS` already contains `localize`/`patch`/`review`/`research`/`orchestrator` (`config.rs:107-109`).
- **R2** Reproduction before patching is enforced structurally, not by prompt: `route_probe` already routes exit-0 to `unexpected_ok` (a gate), so a break that doesn't reproduce escalates. Add a test that proves it.
- **R3** Validation before promotion: `retest` runs the **same** `test_command` in the **same** worktree; only `route_retest.ok` reaches `summarize`/`escalate`.
- **R4** The attempt is written to the ledger with `gate_before`/`gate_after`/`patch_hash`/`model`/`tokens_in`/`tokens_out` (all fields already exist, `crates/pii-store/src/heal.rs:66-116`) — today nothing populates them.
- **R5** An e2e script (`scripts/phase4.7/slice-e2e.sh`) driving `heal-seed → boss-watch detect → spawn → heal-fix → approve → heal-collect → heal-prune`, with the main tree byte-identical to HEAD before and after (`git status --porcelain` empty apart from `_tmp/`).

**TDD contract (acceptance = T3).**
| test | given | expects |
|---|---|---|
| `heal_fix_resolves_patch_model` | `[evolve.routing] patch = "X"`, load real `heal-fix.toml` | the `fix` node's resolved model == `"X"`; a `RouteDecision{step:"patch", model:"X"}` event on the bus (`event.rs:188-195`) |
| `unreproduced_break_escalates` | scripted probe exiting 0 | run suspends on `unexpected_ok`, **no** agent node ever starts (assert the provider call count == 0) |
| `still_broken_after_loop_budget_does_not_reach_gate` | scripted model that never fixes; `loop_budget = 2` | run terminates without `escalate` ever starting, and the ledger row is `Escalated`, not `Fixed` |
| `ledger_records_gate_before_and_after` | scripted successful repair | `HealAttempt{gate_before:"fail", gate_after:"pass", patch_hash: Some(_)}` |

T3 is the one a plausible implementation gets wrong: it will let the retry loop fall through to the human gate with a still-red test, which would put an unvalidated patch in front of an approver — a direct 008 §6 violation.

---

### Spec 34 — Slice F: dev/prod split and a gated, collectable promotion artifact (deterministic, no model)
*Serves*: 008 §3.3, §5 F, §7.1, §7.4.

Scope this honestly. 008 §6 says *"the boss may build and test its own patch, but must never replace the running daemon binary. Deploy stays human, permanently."* So Slice F is **not** "the boss deploys." It is: build the patched version in the worktree, run it as a second daemon against a separate runtime dir, soak it, gate it, and stop at a signed, human-applicable artifact.

- **R1** Dev runtime dir: `[core] runtime_dir` already exists (`config.rs:43`). A dev daemon launched with `runtime_dir = /root/.pii-dev/runtime` gets its own `control.sock`, `store.redb`, `heal.redb`, `approval.redb`, `memory.redb`. Prove isolation: a run started on dev never appears in prod's `run.list`.
- **R2** A `promote-build` xtask step: `cargo build --workspace` **inside the worktree**, then start the built binary against the dev runtime dir, run a fixed soak graph (`graphs/starter/soak.toml`), and record pass/fail. Never touches `/root/.pii/`.
- **R3** The promotion gate is a `gate` node with a **runtime** cycle (`gate.rs:83-87` reads `input.payload.cycle`; spec 26's mechanism), so re-entering the loop mints a fresh `approval_id` (`gate.rs:110-126`) and requires a fresh human decision.
- **R4** On `allow`, the terminal action is `heal-collect` (a `.patch` under `_tmp/heal-reviews/`) plus a `HealAttempt.decision = {allow, approval_id, by, ts_ms}` bound to the gate's id. **Nothing writes the live tree, restarts prod, or merges.**
- **R5** Provenance: the worktree branch is `heal/<sig>` (already so, `heal_seed.rs:81-101`) and the collected patch carries a machine-identifiable trailer, so a human can revert a class of boss changes at once (008 §6).

**TDD contract (acceptance = T2 and T3).**
| test | given | expects |
|---|---|---|
| `dev_runtime_dir_is_isolated` | two engines, distinct runtime dirs; start a run on dev | prod's `run.list` returns 0 rows; dev's returns 1 |
| `promotion_gate_requires_fresh_approval_each_cycle` | two-cycle loop through the promotion gate, cycle 1 approved | cycle 2 **suspends** with a *different* `approval_id` |
| `deny_produces_no_patch_artifact` | promotion gate denied | `_tmp/heal-reviews/<sig>.patch` does not exist; ledger `decision.allow == false` |
| `promotion_never_writes_live_tree` | full promotion cycle in a temp workspace | `git status --porcelain` on the live tree is empty; assert no path outside `_tmp/` was modified |

T3 is the acceptance test — a plausible implementation collects the patch *before* the gate resolves.

---

### Spec 35 — Slice G1: an HTTP fetch capability through the pii-net chokepoint (deterministic, no model)
*Serves*: 008 §3.4, §5 G — and it is a **hard prerequisite the brief doesn't mention**.

There is no HTTP client in this workspace at all: no `reqwest`/`ureq`/`hyper` client in any `Cargo.toml` and `reqwest` is not in `Cargo.lock`. `clippy.toml:12` bans `reqwest::Client::new` in favour of `pii_model::http::shared_client()` — **which does not exist**. `xtask/src/deps.rs:15,30` already forbids `reqwest` reaching the core crates. So the research loop is a new-capability spec, not a graph-wiring spec.

- **R1** One HTTP client behind `pii-net`'s existing grant/chokepoint machinery (`NetSection.grants`, `config.rs:56-60`), single TLS root store, single pool — i.e. actually implement `pii_model::http::shared_client()` so `clippy.toml:12` stops pointing at a phantom.
- **R2** A `fetch` tool in `pii-tools` requiring `CapKind::NET`, host-scoped by the run's grants, with a response size cap mirroring `bash`'s `MAX_OUTPUT = 64 KiB` (`crates/pii-tools/src/bash.rs:101-127`).
- **R3** Gated behind a Cargo feature per `_docs/02-guides/feature-gating.md` (the doc's own argument: uncompiled dialing code cannot be reached).
- **R4** No search-engine dependency: the tool fetches URLs. Where the URLs come from is spec 36's problem (see D6).

**Acceptance**: `fetch_denied_without_net_grant` — a run whose grants don't include the host gets `PiiError::Denied` and **zero** bytes on the wire (assert the client was never dialled, not just the error).

---

### Spec 36 — Slice G2: the research graph writes to memory (needs a model node)
*Serves*: 008 §3.4.

- **R1** `memory` node gains `op = "put"` — today it hard-rejects anything but `"consolidate"` (`crates/pii-server/src/nodes/memory.rs:43-45`), even though the `memory.put` RPC exists (`dispatch.rs:139` → `daemon/methods.rs:231-263`). A graph currently has no way to write memory.
- **R2** `graphs/starter/research.toml`: `timer(cron daily) → fetch → agent(route_step="research") → memory(op="put") → done`. Findings land as `Doc{layer: L1, kind: Atom, evidence: Some((run_id, seq))}` (`crates/pii-memory/src/doc.rs:34-53`).
- **R3** "Try promising tech in DEV" is **explicitly out of scope** — that is spec 34's promotion path with a human at the gate, and wiring research output directly into a patch attempt would violate 008 §6's reproduction requirement.

**Acceptance**: `research_finding_is_retrievable` — after a scripted research run, `memory.search` for a term from the fetched text returns the doc with matching `evidence.run_id`. Not "memory non-empty."

---

## 2. Hard decisions needing a planner/human call before implementation

**D1 — Timer resume mechanism (blocks spec 30).** Option (a): add `fn on_resume(&self, ctx, input, key) -> Option<Outcome>` to the `GraphNode` trait (`crates/pii-graph/src/node.rs:10-16`), default `None`, and have `run::resume` pass the matched `ResumeKey`. Clean and general (a future `wait`/webhook node needs it too), but changes the harness ABI. Option (b): in `run::resume`, synthesise a `NodeFinished{port:"fired"}` for a suspended node whose `SuspendReason::Timer` key matches the wheel entry that fired. Smaller, no ABI change, but hides control flow in the replay path. **Recommendation: (a)** — the ABI is pre-1.0, and (b) puts node semantics in the scheduler, which is exactly the layering mistake that made this bug invisible for two specs.

**D2 — How the boss chains detect→heal (blocks spec 32).** A `spawn` node kind is the only shape that makes "the boss heals" true without a human in the middle of the *mechanism*. The alternative — the human runs `pii run start heal-fix` after approving — is honest but means the boss does not heal, it files tickets. **Recommendation: build the `spawn` node**, keep the human gate *before* it. Confirm this is the intent.

**D3 — Worktree location vs. the dev runtime dir.** These are different things and 008 §7.4 conflates them. `scope_root` must be under the workspace (`runs.rs:261-283`), so worktrees stay at `_tmp/heal-worktrees`. `/root/.pii-dev/` is a *runtime dir* for a second daemon instance (`[core] runtime_dir`), holding its own redb files and socket. Confirm: worktrees under the workspace, `/root/.pii-dev/` as the dev daemon's state only.

**D4 — What "promotion" concretely is.** 008 §6 forbids replacing the running daemon, so the honest terminal is a collected patch plus a green dev-daemon soak, applied by a human. If you want more than that (a `git merge` into `master`, an auto-restart), that contradicts a stated non-negotiable and needs an explicit user override recorded in 008 §7 — do not let an implementer infer it.

**D5 — Reproduction oracle.** Spec 08 already answers this: run the failing test in the worktree and read the exit code (`heal-fix.toml` `probe`/`route_probe`). For the *silent-noop* class that boss-watch actually detects, `heal_detect` already reproduces by running the gate twice (`crates/pii-server/src/nodes/heal.rs:123-165`). But note the mismatch: boss-watch detects `silent-noop-check` failures, while heal-fix repairs a *seeded unit test*. Decide whether Slice E's first live target is (i) the seeded test (008 §7.3, sub-minute, unambiguous — **recommended**) or (ii) a real silent-noop finding (realistic, but the fix is often "wire the thing," which is a design task, not a patch task).

**D6 — Where research URLs come from.** There is no web-search API client and no search dependency. Options: a curated feed list in config (deterministic, cheap, no new provider); an RSS/Atom fetch reusing the existing `rss-gate` vocabulary; or a paid search API (new provider, new secret, new billing surface). **Recommendation: a config-listed feed set for G2**, deferring search-engine integration — it keeps Slice G from becoming a provider-integration project.

**D7 — Does structural separation satisfy "pluggable"?** `pii-evolve` is an unconditional workspace member with no `[features]`, but nothing links it. 008 §3.5 asked for `--features pii-evolve`. Either (i) accept structural separation and **correct `_docs/02-guides/feature-gating.md` and `crates/pii-evolve/src/lib.rs:4-10`** so the doc surface stops asserting a feature gate that doesn't exist, or (ii) add the real Cargo feature. Given 011's central complaint was assertions that aren't true, (i)-with-a-doc-fix is acceptable but must not be silent.

**D8 — Where the boss's own model comes from.** 008 §7.2 says the orchestrator is `glm-5.3:cloud[ollama-cloud]`. `orchestrator` is already a valid routing step (`config.rs:107-109`) but nothing consumes it: boss-watch has no agent node. Decide whether the boss ever gets a model node at the top level, or stays fully deterministic with models only inside `heal-fix`. **Recommendation: stay deterministic** — a model deciding "what to do next" over a two-branch graph buys nothing and burns tokens.

**D9 — Slice C.** Not in the brief's list, but unbuilt and user-requested. `compaction.rs:69` still can't fire, and `state.md` is written outside the run. If context management matters for this phase, it is a spec; if not, say so in 008 §9 rather than leaving the claim standing. **Reference design when we build it**: `013-dsh-context-slice-c-reference.md` (dsh-context plugin — context insight as a projection over the existing journal, read the harness's own token accounting, delta-mode diagnostic for the compaction no-op).

---

## 3. Ordering, dependencies, and model use

| # | Spec | Depends on | Model? | Risk |
|---|---|---|---|---|
| 30 | timer resume | — | **No** | Low code, high leverage. Nothing else works without it. |
| 31 | protected paths | — | **No** | Low. Must precede 33; parallelisable with 30. |
| 32 | spawn + provision | 30, 31 | **No** | Medium — new node kind, new run-linkage event. |
| 33 | Slice E live | 32, 31 | **Yes** (`localize`, `patch`) | Highest — first live model with write capability. |
| 34 | Slice F dev/prod | 33 | **No** | Medium — process/build orchestration, no AI. |
| 35 | Slice G1 fetch | — | **No** | Medium — new dependency + net surface. Independent of 30–34. |
| 36 | Slice G2 research | 35 | **Yes** (`research`) | Low once 35 lands. |

30 and 31 can run in parallel; 35 is independent of the whole heal chain and can be slotted whenever there is capacity. Do not start 33 before 31 is green — that is the one ordering where getting it wrong lets a model delete the test that proves it worked.

Also worth folding somewhere: 011's "Optional 30" residuals (`compaction.rs:69`, `service.rs:292` lease leak, `dispatch.rs:205` unfiltered `run.list`, `session_durability.rs:38`). None blocks the boss; `run.list` filtering becomes relevant once spec 34 runs two daemons.
