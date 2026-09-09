# Pii (`pi-improved`) — architecture-conformance review + next-phase proposal

**Reviewer**: claude-opus-5 · **Target**: `/projects/pi-improved` @ `648bf56` (master, clean)
**Ground truth**: `_docs/06-research/001`, `002`, `003` (architecture), `007` (Phase-4 plan), `008`/`009` (boss design + prior review), `010` (prior full review)
**Method**: every claim below was checked against the code in this tree. No edits, no commits, no builds.

## Verdict

**NEEDS WORK** — the substrate still conforms and spec 22 genuinely fixed the tool loop, but `pii-evolve` (spec 23) is the first module in this repo whose doc comments assert properties the code does not have, and the promotion prerequisites 009/010 flagged as blocking are all still open.

---

# Part A — Conformance to the initial architecture

## A.0 Where the code follows the plan (verified, not assumed)

- **Capability substrate (001 §2.4)**: `Ctx` carries cancel/deadline/budget/caps/principal/trace/obs (`crates/pii-core/src/ctx.rs:20-33`); `CapSet` has `kinds`/`fs_read`/`fs_write`/`net_hosts`/`proc_argv0`/`secrets` (`crates/pii-core/src/cap.rs:121-134`); accessors return `Result` so the check happens at the accessor (`ctx.rs:172`, `:185`, `:197`); `child()` intersects rather than unions (`ctx.rs:209`). This is the plan, built.
- **Dependency DAG (002 §12.2)**: `xtask/src/deps.rs` implements the four invariants plus two more (redb confined to `pii-store`, `deps.rs:33`; rustls confined to `pii-net`, `deps.rs:14`). Invariant 2 holds — `crates/pii-proto/Cargo.toml` has no `pii-core`. Invariant 3 holds *for `pii-tui`* (see A11).
- **Pass-3 scheduler semantics (003 §4.4)**: `JoinPolicy::{All, Any, Any:cancel}` exists (`crates/pii-graph/src/spec.rs:17-40`) — the `Race`-node simplification landed as specified.
- **Spec 22 landed and is real.** `chat_streaming` accumulates `message.tool_calls` across NDJSON lines with a cross-chunk `line_buf` (`crates/pii-model/src/ollama.rs:194-216`) and a test that splits a tool-call line mid-JSON (`ollama.rs:620`); the caller's `Ctx` is cloned into the timed tool thread instead of `CapSet::empty()` (`crates/pii-server/src/agent.rs:412-416`); `request_streaming` dechunks incrementally against a real chunked server (`crates/pii-net/src/service.rs:399-440`, test `:627`); `message_value` emits `tool_calls`/`tool_call_id` (`ollama.rs:311-336`); `run_ci` now enforces both self-improvement gates (`xtask/src/main.rs:66-68`, asserted by `main.rs:299`); the MCP server has a bounded lifetime via `running.waiting()` (`crates/pii-mcp/src/main.rs:154`). Provider options reach the wire (`ollama.rs:95-97`). Child processes get their own process group and a scrubbed env (`crates/pii-core/src/childproc.rs:45`, `:197`, `:302`).

That is the good news, and it is substantial. Everything below is what did not follow.

---

## A.1 — `pii-abi` is not the ABI: none of the plan's core traits live there

001 §2.5 and 002 §12.1 both place `Tool`, `ModelProvider`, `Channel`, `Memory` and `Observer` in `pii-abi` ("ABI STABILITY. Traits + types + ids + errors"). Today `crates/pii-abi/src/lib.rs:8-22` exports types only — cap, effects, error, http, ids, journal, node. The traits are scattered: `Tool` at `crates/pii-tools/src/lib.rs:25`, `ModelProvider` at `crates/pii-model/src/lib.rs:115`, `Channel` at `crates/pii-channels/src/lib.rs:33`, `Observer` at `crates/pii-core/src/observe.rs:40`, `GraphNode` at `crates/pii-graph/src/node.rs:10`.

**There is no `Memory` trait at all.** Consequently `pii-server` binds a concrete store: `memory: Option<Arc<pii_store::MemStore>>` (`crates/pii-server/src/agent.rs:33`). 001 §6.1's closing sentence — "Keep the `Memory` trait clean so a SQLite backend remains a drop-in if this proves painful" — is no longer available. Silent drift; I found no doc recording it as a decision.

## A.2 — `top_k` is a declared graph key with zero readers

`graphs/starter/assistant.toml:18` declares `memory = { enabled = true, top_k = 6 }`. `enabled` is genuinely read (`agent.rs:94-100` — 007's complaint fixed). `top_k` is read nowhere in the workspace: the searcher hardcodes `6` (`agent.rs:141`) and the injector hardcodes `.take(6)` (`agent.rs:146`), and `TrigramEmbedder` is hardcoded too (`agent.rs:139`). The declared value happens to equal the hardcoded one, which is exactly why nobody noticed. Set it to 20 and nothing changes. This is 007 §3 R3's class, still live.

## A.3 — Five RPC methods declared on the wire with no dispatch arm

`Method::SessionInterrupt`, `SessionClose`, `GraphApply`, `GraphGet`, `GraphList` (`crates/pii-proto/src/method.rs:29`, `:38`, `:59`, `:62`, `:65`) have **zero** `Method::X =>` arms in `crates/pii-server/src/dispatch.rs`; all five fall to the catch-all at `dispatch.rs:145-148` → `MethodNotFound`. `GraphApplyParams`/`GraphApplyResult` are fully-defined wire types with no server (`crates/pii-proto/src/run.rs:84`, `:93`). 002 §3.5 defines the method surface as the contract. Spec 16 fixed `session.list` and left the other five.

## A.4 — Three `Event` variants with no producer

`Event::WorkerDied` (`crates/pii-proto/src/event.rs:155`), `Event::WorkerRestarted` (`:162`) and `Event::MemoryDegraded` (`:180`) appear only in `subscribe.rs` filters and `pii-tui` renderers. The complete set of broadcast sites in the workspace is `ApprovalRequested`/`ApprovalResolved` (`crates/pii-server/src/daemon/approvals.rs:64`, `:29`), `ChannelMessage` (`crates/pii-server/src/daemon/router.rs:104`), and the `RunEvent`→`Event` mapping in `bus.rs`. `pii-worker`'s supervisor restarts workers and never broadcasts; `MemorySearchResult` carries a `degraded` flag (`crates/pii-memory/src/search.rs:35`) that never becomes an event. `AssistantDelta` was 007's example and is now really produced — three siblings of the same shape remain (002 §8.4 defines the taxonomy as the observable contract).

## A.5 — `run.list` still returns `[]` without a status filter

`crates/pii-server/src/dispatch.rs:205`: `let runs = status.map_or_else(Vec::new, |st| engine.store.runs_by_status(st)...)`. 007 §1 listed this in the "system lies about itself" table. Unfixed. `pii run list` on a healthy daemon shows nothing.

## A.6 — `Ctx::check()` exists; the agent turn loop never calls it

`ctx.rs:289` defines it. There are **zero** `ctx.check()` call sites in `crates/pii-server/src/agent.rs`. 007 P4.1 required "`ctx.check()?` at the top of each turn and between tool calls". Today a `run.cancel` during an 8-turn agent node is not observed until the node returns: `childproc.rs:302` kills a running `bash`'s process group, and `crates/pii-graph/src/sched/dispatch.rs:245` handles a cancelled error after the fact, but the model call and the turn boundary (`agent.rs:155`) and the tool-dispatch boundary (`agent.rs:355`) are blind. For a boss loop this is the difference between "stop it" and "wait for it".

## A.7 — Three of four budget dimensions are never charged

`BudgetKind::{ToolCalls, MicroUsd, WallMs}` (`crates/pii-core/src/budget.rs:11-18`) have no `charge` site anywhere. The workspace's only charge is `agent.rs:217` (`Tokens`). Worse, `crates/pii-graph/src/run.rs:265` *reads* `MicroUsd` remaining to route the `budget` port — a port that can never fire, producing an error string (`run.rs:268`) that can never be seen. `max_model_calls` still exists only in `specs/06-phase3-mvp-hardening.md`. 001 §2.4 specifies "tokens, wall-time, tool-calls, USD spend — atomics". Unchanged since 010 B2.

## A.8 — Four redb files, not one

002 §12.1: `pii-store` is "THE ONLY CRATE THAT KNOWS redb (**pii.redb**)"; 001 §3.4/§6.2 lay out one schema. `Engine::new` opens four databases — `store.redb`, then `approval.redb` (`crates/pii-server/src/lib.rs:141`), `heal.redb` (`:144`), `memory.redb` (`:145`). The *crate-ownership* invariant holds (`deps.rs:33`); the *file* invariant does not.

The consequence is load-bearing for P4.7, not cosmetic: a heal cycle's journal event, its ledger row and its approval decision cannot be one ACID transaction. A crash between them leaves the boss's audit trail inconsistent — which is precisely the record a human is supposed to approve from. This looks like a deliberate response to the rss-gate cache-footprint problem (each auxiliary store sets a 4 MiB cache, `crates/pii-store/src/heal.rs:52-57`), but I found no doc amending §12.1, so it reads as drift rather than a decision.

## A.9 — `SiteId` and HLC are absent — the one thing 002 said must be done early

002 §7.5: "`SiteId` must be in every syncable key or record. Retrofitting it is a migration, so put it in from Phase 1 even though sync ships in Phase 6. **This is the one thing that must be done early.**" `grep -rn "SiteId\|site_id\|hlc" crates/` returns nothing. Deferring `pii-sync` was correct; skipping the cheap prerequisite that made deferral safe was not. This is the most expensive silent divergence in the tree — every table written since Phase 1 now needs a migration whenever sync arrives.

## A.10 — `pii-obs` never existed

002 §12.1 lists it and §12.3 puts it in the **initial seven** with an explicit argument: "observability retrofitted is observability that never happens, and `record!` call sites are cheapest to write when the code is written." `Observer` lives at `crates/pii-core/src/observe.rs:40`. Defensible in outcome (one fewer crate, the seams exist), but it contradicts an explicitly-argued decision with no counter-argument on record. Classify as drift, low severity.

## A.11 — `pii-web` breaks the client/server split the plan made machine-checkable

002 §12.2 invariant 3 is stated as a proof obligation: "`pii-tui` must not have `pii-graph` in its dependency tree. This is the machine-checkable proof that the client/server split (§3.6) is real and not aspirational." `pii-tui` complies exactly (`crates/pii-tui/Cargo.toml`: `pii-proto[client]`, ratatui, crossterm, tokio).

`crates/pii-web/Cargo.toml:4` takes **`pii-server` as a direct dependency**, pulling `pii-graph`, `pii-store`, redb and the entire daemon tree into a read-only viewer — and it does so for exactly two helper functions: `pii_server::uds::client_connect` (`crates/pii-web/src/main.rs:51`) and `pii_server::uds::runtime_dir` (`:205`). `deps.rs:29` hardcodes `TUI_FORBIDDEN` to `pii-tui` alone, so the gate is structurally blind to it. The invariant is now false for the newest client, and the enforcement did not follow the architecture. 007 P4.6 explicitly specified the opposite shape ("proxy the daemon's UDS JSON-RPC over loopback HTTP; separate `pii-web` crate keeping axum out of core").

## A.12 — `OllamaEmbedder` is still unreachable and still reports `dims() == 0`

`crates/pii-memory/src/embed.rs:126-128` returns `0 // unknown until the first response`. Its only mention outside its own file is the re-export at `crates/pii-memory/src/lib.rs:19`. 007 §5 listed it under "cut or fix (things that currently lie)"; two review cycles later, neither happened. `reachability-check`'s `SEED` (`xtask/src/reachability.rs:47-58`) contains two hand-written entries and does not cover it.

## A.13 — Two declared config keys with no readers

`crates/pii-server/src/daemon/config.rs:110-114` documents `[memory] embedder` and `[memory] cache_bytes` as "reserved". `cache_bytes` has zero occurrences outside its own declaration and default. `embedder` never selects anything (the agent hardcodes `TrigramEmbedder`, A.2). The module's own header states the design intent it violates: "`deny_unknown_fields` throughout — a typo'd key must be a load error, not a silently-ignored no-op" (`config.rs:1-2`). A *known* key with no reader is the same failure with the gate's blessing.

## A.14 — `pii-evolve` documents itself as feature-gated and is not

`crates/pii-evolve/src/lib.rs:4-8`: *"A dedicated, **feature-gated** crate: it is not a dependency of `pii-cli` or `pii-server`, so the default workspace build links none of it — an installer that does not want self-improvement builds no boss code and spends no boss tokens."*

Half of that is true (nothing depends on it) and half is false. It is an **unconditional workspace member** (`Cargo.toml:18`), and there is no Cargo feature named `evolve` or `pii-evolve` anywhere in the workspace — `grep '\[features\]'` across all manifests finds only `pii-net`, `pii-proto` and `pii-server` (whose sole feature is `testkit`). `cargo build --workspace` and `cargo test --workspace` build it today.

008 §3.5 is unambiguous: *"Off (the default) = nothing of `pii-evolve` is compiled or linked; the binaries are byte-identical to today. On = ... `cargo build --features pii-evolve`."* The user's explicit pluggability requirement is unimplemented, and the code asserts it is done. This is the single most on-the-nose instance of the exact pathology the boss is being built to detect — and it is inside the crate written to build the detector.

## A.15 — The boss does not run on the substrate it heals

008 §3/§4 and 009 both make this the crate's central claim: *"The boss runs on the substrate it heals — a pii graph, using pii's own journal, gates, approvals, budget, memory, and model hierarchy. Not a bolt-on service."*

`crates/pii-evolve/Cargo.toml` has exactly one dependency: `pii-core`. No `pii-graph`, no `pii-store`, no `pii-server`. `driver::run_cycle` (`crates/pii-evolve/src/driver.rs:88-127`) is a hand-rolled `for` loop that shells out to `cargo xtask` through `ctx.proc()`, writes a Markdown file, and returns. There is no journal event, no `gate` node, no `ApprovalStore` row, no `HealStore` row, and the budget is `Budget::unlimited()` (`driver.rs:141`) against 008 §6's non-negotiable "budget-bound every attempt".

`CycleOutcome::PendingHuman` (`driver.rs:75`) — documented as "suspends for a human, never self-approves" — writes `needs_attention: true` into `state.md` (`driver.rs:110-112`) and exits 1 (`crates/pii-evolve/src/main.rs:56-59`). **Nothing creates an approval and nothing can resume it.** The `pii approve` path that the user's #1 resolved decision (008 §7.1) depends on is not connected to this code at all.

## A.16 — Two of `pii-evolve`'s three public modules are dead

`routing::{RoutePolicy, RouteLog, Tier}` and `context::{CompactionPolicy, TranscriptEntry, summarize_for_compaction}` are referenced nowhere in `crates/` except their own test files (`crates/pii-evolve/tests/routing_tests.rs`, `tests/context_tests.rs`). `driver.rs` — the only module that actually runs — imports neither. `StateDoc::summarize()` (`crates/pii-evolve/src/state.rs:80`) likewise has only a test caller. So 008 §3.1 (context management) and §3.2 (model routing) are shaped, tested, and inert: spec 23 shipped the parts without the wiring.

`reachability-check` structurally cannot catch this — a `pub` item in a library crate is reachable by definition. That is exactly why 008 §5 Slice A asked for the gate to be extended, and Slice A has not been built.

## A.17 — Every promotion prerequisite from 009/010 is still open

Verified unfixed at `648bf56`:

| # | Defect | Location |
|---|---|---|
| a | **Approval gate auto-allows inside a loop.** `cycle` is read once from static graph config at build time (`gate.rs:56-60`); nothing increments it at runtime. A gate node re-entered on iteration 2 of the same run computes the same `approval_id` (`gate.rs:78`), finds its own cached decision (`gate.rs:79-87`) and returns `allow` with no human. This is the boss loop's exact topology. | `crates/pii-server/src/nodes/gate.rs:56,78-87` |
| b | **`spawn_resume` restores `scope_root` but never re-validates it.** `validate_scope_root` is called only from `spawn_run`. | `crates/pii-server/src/runs.rs:43` vs `:138` |
| c | **Connection-lease leak on TLS setup failure.** `dial_tls(...)?` returns after the lease increment and before `Conn` is built, so `Drop for Conn` never runs and the dial-error decrement is off-path. `max_conns` such failures permanently blackhole that `(principal, key)`. | `crates/pii-net/src/service.rs:283`, `:292`, `:296`, `:133-140` |
| d | **Compaction still cannot fire on an agent transcript.** A turn is counted as a `Role::User` message (`compaction.rs:69`); one agent run has exactly one, so `n_turns == 1 <= keep_recent_turns` (6) and `compact` returns its input unchanged (`:75`). `needs_compaction` keeps returning true; nothing ever shrinks. Secondarily, `estimate_tokens` sums only `m.content` and ignores `tool_calls` arguments — the largest payloads in a tool loop are invisible to the estimate. | `crates/pii-server/src/compaction.rs:35-41,69,75` |
| e | **No `HealStore` schema evolution.** Byte-for-byte the Slice-0 shape: stringly-typed `outcome: String`, no `patch_hash`, no `gate_before`/`gate_after`, no `decision`, no cost fields, unbounded `attempts: Vec`. 010 §C's recommendation is entirely unimplemented. | `crates/pii-store/src/heal.rs:13-38` |
| f | **No retention/prune.** `grep prune crates/ --include=*.rs` outside heal-signature code: nothing. `pii admin prune` (007 P4.2) was never built. | — |
| g | **No anti-reward-hacking refusal.** No code in `crates/` or `xtask/` refuses a patch touching `xtask/`, `quality-gate.sh`, `graphs/`, `specs/` or a `#[test]`. | — |
| h | **Session persistence still O(n²).** `persist_session` re-serialises the entire transcript and is called on every assistant turn; token accumulation is still a non-atomic read-modify-write. | `crates/pii-server/src/session_durability.rs:38-44,97,129` |

## A.18 — Intentional deviations (correctly separated from drift)

These are recorded decisions, not defects: the `pii-evolve` crate itself (user override, 009); `pii-mcp` and `pii-web` as new crates (007 P4.6, spec 21); `pii-wasm`/`pii-hw`/`pii-acp` not built (007 §5, phase deferral); `JoinPolicy::Any` replacing the `Race` node (003 §4.4, implemented); the rustls-confinement invariant added beyond the plan's four (spec 17 — an improvement on the plan). `CapSet`'s collapse of `HostScope`/`ArgvScope`/`SecretScope` into one `StringScope` (`cap.rs:129-133`) is naming drift, low severity — but see the cut list, because the plan's three names encoded three different matching semantics.

---

## A.19 — Verdict on the whole

007's "**production-grade substrate carrying an MVP-grade product**" is still true, and it now needs one amendment.

The *tool loop* is no longer MVP-grade — spec 22 fixed the three defects that made every autonomous run a clean, successful no-op, and it did so with tests that model the wire rather than fakes that bypass it. That was the right call and it was executed well.

What replaced it is a narrower and more dangerous shape:

> **The substrate is production-grade. The product is roughly beta-grade. The self-improvement layer built on top is demo-grade while documenting itself as production-grade.**

Every prior instance of the silent-no-op class in this repo was an *omission* — a wire nobody ran. `pii-evolve` is the first module that makes *assertions*: feature-gated (it isn't, A14), suspends for a human (it doesn't, A15), runs on the substrate it heals (it doesn't, A15). That is a category change. An omission costs a wire; an assertion costs trust in the doc surface that the whole review process reads from — and this codebase's coherence has been carried by the doc surface being honest.

---

# Part B — Next phase: specs 24–29

Ordering rationale: the deterministic gate first (it is the boss's fitness function, it is free, and it would have caught A2/A3/A4/A13 automatically), then `pii-evolve`'s honesty, then the promotion prerequisites, then the graph-native loop, then routing, then the brake.

### 24 — `xtask silent-noop-check`: the declared-but-unwired gate
*Serves*: 008 §5 Slice A; 007 §4 P4.7 ("the highest-leverage self-improvement work"); catches A2, A3, A4, A13.

**Goal**: one deterministic, no-model xtask subcommand that fails CI when a declared surface has no consumer, in three classes: (a) every `Method` variant in `pii-proto/src/method.rs` has a `Method::X =>` arm in `pii-server/src/dispatch.rs` or an allowlist entry with a written reason; (b) every `Event` variant has a construction site outside `pii-proto`, `subscribe.rs` and `pii-tui` (filter and render sites are not producers); (c) every field of every `Deserialize` config section in `daemon/config.rs`, and every node-config key in `graphs/starter/*.toml`, has a read site.

**Exit criterion (must fail first)**: on `648bf56` the gate reports **exactly these 11** and exits 1 — `session.interrupt`, `session.close`, `graph.apply`, `graph.get`, `graph.list`, `Event::WorkerDied`, `Event::WorkerRestarted`, `Event::MemoryDegraded`, `[memory] cache_bytes`, `[memory] embedder`, `assistant.toml top_k`. Each is then wired or allowlisted with a reason, and the gate goes green. Wire into `run_ci` beside reachability/doc-drift (`xtask/src/main.rs:66-68`). **A unit test must prove the gate goes red when a fabricated method variant loses its arm** — otherwise it is another vacuous pass, which is how spec 16 part 2 and spec 20 got through.

### 25 — `pii-evolve` honesty: actually pluggable, and no dead modules
*Serves*: 008 §3.5; 009 user override; fixes A14, A16.

**Goal**: make the crate's doc comments true. Add a workspace `evolve` feature and make `crates/pii-evolve` optional so a default build links none of it. Then either wire `context.rs` and `routing.rs` into `driver::run_cycle` or delete them and re-add in spec 27/28 — do not ship a third pass of shaped-but-dead code. `run_cycle` takes a real `Budget`, not `Budget::unlimited()` (`driver.rs:141`).

**Exit criterion**: (1) `cargo build --workspace` yields a `pii` binary whose `blake3` matches the pre-change build, and `--features evolve` additionally yields `pii-evolve`; (2) spec 24's gate, extended to flag `pub` items in `pii-evolve` with only test callers, reports zero; (3) a test asserts `run_cycle` returns `BudgetExhausted` under a zero-gate-call budget.

### 26 — Promotion prerequisites: runtime approval cycle + a ledger worth approving from
*Serves*: 008 §6/§7.1; 009 items 1–2; 010 E2 and §C. Fixes A17a, A17b, A17e.

**Goal**: a human approval inside a loop is a fresh decision, and there is something to approve *from*.
- Derive the gate salt at runtime — fold the loop-iteration counter (or an explicit `cycle` from `input.payload`) into `approval_id` (`gate.rs:56-60`, `:78-87`), keeping the config field's default of 0 so §8.8 replay is byte-identical.
- Evolve `HealAttempt`/`HealAttemptRun` (`heal.rs:13-38`) per 010 §C: a `HealState` enum replacing `outcome: String`; `patch_hash` content-addressed through the existing CAS (never inline); `worktree`/`branch`/`base_commit`; `gate_before`/`gate_after`; `decision { allow, approval_id, by, ts_ms }` bound to the id `gate::approval_id` mints; `model`/`tokens_in`/`tokens_out`/`micro_usd`; `attempts` as a bounded ring (default 20) with a separate `attempt_count`.
- While in `runs.rs`: call `validate_scope_root` from `spawn_resume` (two lines, A17b).

**Exit criterion**: a test drives a two-iteration loop graph through one gate node and asserts iteration 2 **suspends** with a *different* `approval_id` rather than returning `allow`. A second test round-trips a `HealAttempt` with `gate_before="fail"`, `gate_after="pass"` and a `decision` whose `approval_id` equals the gate node's.

**Why before 27**: 008 §6 makes "never auto-promote" non-negotiable and 010 B4 proves it does not hold in the boss's topology. Building the loop first ships an unapprovable promotion path.

### 27 — The boss loop as a real pii graph (008 §5 Slice B, done properly)
*Serves*: 008 §3, §5; fixes A15.

**Goal**: replace `driver::run_cycle`'s hand-rolled loop so the dogfooding claim is true.
- `graphs/starter/boss-watch.toml`: `timer` → `heal_detect` (gate = spec 24's `silent-noop-check`) → `route` → `gate` (fresh cycle from spec 26) → `transform`. Model it on `heal-watch.toml`, whose shape is already right.
- `pii-evolve` gains `pii-graph` + `pii-store` behind the `evolve` feature and registers a `state_write` node kind, so `state.md` is written *by a node inside the run* and the journal is the audit trail.
- `CycleOutcome::PendingHuman` becomes a real `Outcome::Suspend` with an `ApprovalStore` row.

**Exit criterion**: seed one of spec 24's silent no-ops back in, run `boss-watch`, and `pii graph trace <run_id>` shows `NodeStarted`/`NodeFinished` for detect plus a `RunSuspended` carrying an `Approval` reason; `pii approve list` returns the row; `pii approve <id> --deny` terminates the run on the `deny` port. **No model call anywhere in this spec.**

### 28 — Model routing wired into the agent layer
*Serves*: 008 §3.2; 009 item 6 as amended by the user override; fixes A16.

The wiring point is the **agent node's config**, not a new node kind — 009 item 6 was right about that, and the user's override concerned crate placement, not node kinds. `AgentNode` already reads an optional model (`agent.rs:162-165`); add a `route_step` key resolved through a `RoutePolicy` loaded from `[evolve.routing]`, and record each `RouteDecision` into the journal so `pii graph trace` shows it. The 008 §3.2 escalation rule ("escalate after 2 attempts") reads the `attempt_count` spec 26 adds.

**Exit criterion**: a graph with two agent nodes declaring `route_step = "patch"` and `route_step = "review"` resolves to two *different* model specs from one config, both visible in `pii graph trace`. A second test asserts an unassigned step fails the node with `NotFound` rather than silently defaulting — 008 §1 is explicit that a single misroute cascades, so it must be loud.

### 29 — Cancellation and the brake
*Serves*: 001 §2.4; 007 P4.1 R4; fixes A6, A7.

`ctx.check()?` at the turn head (`agent.rs:155`) and before each tool dispatch (`agent.rs:355`). Charge `BudgetKind::ToolCalls` at `agent.rs:355` and `WallMs` at the turn boundary; charge `MicroUsd` from provider cost metadata **or** delete the dimension together with the unreachable read at `run.rs:265`. Add `max_model_calls` to `GraphLimits` and enforce it in the loop.

**Exit criterion**: a test cancels a run mid-tool-loop and asserts the node returns `Cancelled` *before* the next model call (assert the scripted provider's call count, not just the terminal). A second asserts a graph with `max_tool_calls = 2` routes to the `budget` port on the third call. Both tests must fail if the `check()`/`charge()` lines are removed.

### Optional 30 — the residual 010 defects, one pass
`compaction.rs:69` turn-splitting (A17d), `service.rs:292` lease leak (A17c), `dispatch.rs:205` unfiltered `run.list` (A5), `session_durability.rs:38` write amplification (A17h). Each is small and they share no design question, so grouping is safe. Fold in only if 24–29 leave capacity; none of them blocks the boss.

---

## Cut / fix / rename list

1. **Delete `OllamaEmbedder`** (`crates/pii-memory/src/embed.rs:103-140`). Two review cycles have listed it; no caller, `dims()` returns 0 so the vector lane can never select it, and trigram is what ships. If a semantic lane is wanted later, write it against a real dimension probe. Cheaper to delete than to keep explaining.
2. **Move `pii_server::uds::{client_connect, runtime_dir}` into `pii-proto[client]`** (or a ~100-line `pii-uds`), drop `pii-server` from `crates/pii-web/Cargo.toml:4`, and generalise `deps.rs:29`'s `TUI_FORBIDDEN` into a `CLIENT_CRATES` list covering `pii-tui` **and** `pii-web`. Restores 002 §12.2 invariant 3 as a proof about clients rather than a proof about one crate.
3. **`BudgetKind::MicroUsd`** — charge it (spec 29) or delete it *with* `run.rs:265`. Do not leave a third review to rediscover it.
4. **`[memory] cache_bytes` / `[memory] embedder`** (`config.rs:110-114`) — delete the keys or read them. `deny_unknown_fields` makes deletion a hard load error for anyone relying on them, which is the honest outcome.
5. **`Method::{SessionInterrupt, SessionClose, GraphApply, GraphGet, GraphList}`** — implement or remove from `pii-proto`. `graph.apply`/`graph.get`/`graph.list` are the ones the boss will need (it installs its own graphs); `session.interrupt`/`session.close` should be implemented, since `run.cancel` and session lifecycle already exist behind them.
6. **`crates/pii-evolve/src/lib.rs:4`** — the phrase "feature-gated crate" must not survive spec 25 unless the feature exists. Extend `doc-drift-check` to flag a doc comment naming a Cargo feature that `cargo metadata` does not list; it is the same one-class-of-mismatch discipline the existing check already uses.
7. **Rename `CapSet`'s scope types** (`cap.rs:129-133`). 001 §2.4 specified `HostScope`/`ArgvScope`/`SecretScope` because they encode three different matching semantics — wildcard host+port, program basename, namespace prefix. One `StringScope` means one semantics. It is free to split now and a migration once host wildcarding or port matching lands.
8. **`pii-evolve`'s `--policy FILE`** (`crates/pii-evolve/src/main.rs:37-51`) — a plain-text file whose entire contents are a directory path is a strange contract for a crate whose job is auditable policy. Make it spec 28's TOML routing/gate policy, or drop the flag for a positional workspace root.

**Do not touch**: the 450-effective-line gate (`xtask/src/lint_files.rs:11`), the lint set, `deps-check`, `net-check`, the Landlock fail-closed self-probe, `panic = "abort"`. 007 §5 was right and nothing since has weakened the argument.

---

### One-paragraph honest summary

The engine still conforms to 001–003 where it matters — capabilities, journal, scheduler semantics, egress chokepoint, dependency purity — and spec 22 did the hardest and most valuable thing in this phase by fixing the agent tool loop against a real provider with tests that model the wire instead of bypassing it; credit there is earned. But the drift that 007 named and 010 confirmed has not been paid down: three of four budget dimensions are decoration, five RPC methods answer `MethodNotFound`, three event variants have no producer, `run.list` returns nothing, `ctx.check()` is never called from the loop it was written for, `SiteId` — the one thing 002 said must land in Phase 1 — is absent, and every promotion prerequisite from 009/010 is still open. The new development is `pii-evolve`, which is the first module here to *assert* properties it does not have: it says it is feature-gated (no such feature exists), that it suspends for a human (it writes a Markdown flag and exits 1), and that it runs on the substrate it heals (it depends on `pii-core` alone, and two of its three public modules have no caller but their own tests). The substrate is production-grade, the product is beta-grade, and the self-improvement layer is demo-grade while documenting itself otherwise — so the next phase should not add capability. Build the silent-no-op gate first because it is deterministic, free, and would have caught four of these findings by itself; make `pii-evolve` true before making it bigger; land the runtime approval cycle and the heal-ledger evolution before anything can be promoted; and only then let the boss be a real graph.
