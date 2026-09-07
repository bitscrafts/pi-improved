# Pii — Phase 4: "Daily Driver" (MVP-complete review + next-phase plan)

**Date**: 2026-09-07 · **Author**: claude-opus-5 (analysis) + orchestrator (write-up) · **Status**: PLAN — spec-sized items to be authored/implemented via pidag on greenlight

## 1. Executive verdict

**Pii is a production-grade *substrate* carrying an MVP-grade *product*.**

The layers nobody sees are the good ones: the graph engine, the journal/replay model, the three-layer capability intersection, the Landlock sandbox with its fail-closed self-probe, the redb store with batched write-behind and flush barriers, and the mechanically enforced architectural chokepoints (`xtask deps-check`, `net-check`, `reachability-check`, `doc-drift-check`, "one `Command` site", "one `TcpStream::connect` site", "one redb writer"). That is the moat, and it was earned.

The layer users actually touch — `pii-server::agent`, sessions, the tool set, the model provider — is thin, and it is where every embarrassment lives. The dominant pathology is **features that are shaped but not wired**:

| Declared | Reality |
|---|---|
| `memory = { enabled = true, top_k = 6 }` in `assistant.toml` | parsed by nobody |
| `Event::AssistantDelta` — rendered by the TUI | **never emitted**; `ollama.rs:85` sends `"stream": false` |
| `memory.search` returns `lanes: ["bm25","vector","graph"]` | `memory.put` writes `vector: None` — the vector lane is empty for every doc the product writes |
| `OllamaEmbedder` — "the real backend" | `dims()` returns `0`, so the vector lane can never run |
| `max_micro_usd` budget dimension | checked but **never charged** |
| `session.list` in the wire protocol | falls through to `MethodNotFound` |
| `run.list` | returns `[]` unless a status filter is passed |
| `session.state` token counts | hardcoded `tokens_in: 0, tokens_out: 0` |

None of these is hard to fix. All of them make the system *lie about itself* — the specific failure mode that erodes trust fastest in daily use.

**Recommendation: do not add a subsystem. Bring the agent layer up to the standard of the engine, then make the daemon survive its own restart, then wire memory, then TLS, then the web viewer.** Self-improvement (real PR-opening) should stay parked; the honest minimal self-improvement loop is cheaper and safer than it looks, and it does not need the network.

## 2. What's solid vs what's MVP-grade (ranked)

### Genuinely solid — leave alone, build on
1. **The graph engine and its journal** (`pii-graph/src/{run,journal,sched}`). Event-sourced, batch-drain tick loop, effects-partitioned parallelism, Tarjan cycle validation with `loop_budget`, replay-based resume, bounded write-behind with flush barriers, `NodeInterrupted` markers. The `loop_budget_exceeded_fails` race fix was a real engine bug found by hardening — a healthy signal.
2. **The capability model.** Three-layer intersection (run ∩ graph ∩ node), `PathScope` with `dunce::canonicalize`, `validate_scope_root` rejecting `..`/symlink escapes, tests that assert *denial*. The Slice-1 heal confinement (fix agent scoped to a disposable worktree, no `bash`) is the correct shape for agentic safety.
3. **Mechanical architecture enforcement.** `deps-check`'s five DAG invariants, `net-check`'s single-connect allowlist, 450-line file cap, `-D warnings` with `unwrap_used`/`panic`/`indexing_slicing` denied. This is why a 27k-line codebase written largely by cheap models is still coherent.
4. **The Landlock sandbox.** Real ABI V2 ruleset, requires `FullyEnforced && no_new_privs`, proves confinement with a write probe that must fail. Fail-closed, correctly.
5. **The store.** redb ACID, content-addressed graph specs, header+journal+snapshot separation, memory index in a separate DB, `append_events` batching.
6. **Transport hygiene.** peercred uid check, 0700 dir + 0600 socket, hello gate with hard version-skew refusal, 5-minute idle timeout, real `event.unsubscribe` cancellation, bounded subscriber queues with `Lagged`.

### MVP-grade scaffolding — the next phase's work
1. **The agent turn loop** (`agent.rs`). No streaming; no compaction; no `ctx.check()` inside the loop (a `run.cancel` during a model call or a 10-minute `bash` is ignored until the node returns); one blanket retry; tool results flattened to strings with `files_changed` inferred by matching tool *names*.
2. **Sessions.** RAM-only; ids `format!("s{}", len+1)` collide by construction if a session is removed; transcript `Vec<WireMessage>` grows without bound and is *cloned in full* every turn; no `session.list`; no persistence; token counters faked.
3. **Memory.** Substrate excellent, product absent — and worse than "not wired": the write RPC omits vectors, so even after spec 09 the vector lane would return nothing while reporting itself healthy. No `memory.get` — `memory.search` returns doc *ids* only.
4. **The tool set.** Five tools; no `grep`/`glob`/`ls` — a small local model burns turns doing `bash: find . -name`. The deny-list is substring matching (`"sudo"` blocks `sudoku`; `rm -fr /` sails past `rm -rf /`) — harmless as defense-in-depth, dangerous if believed to be the control.
5. **Child-process handling** (`ctx.rs:538-601`). No process group (a timed-out `bash` orphans the `cargo test` beneath it); full parent environment inherited (the model's `bash` can read every API key the daemon holds); no cancellation observation.
6. **Live-model integration.** `temperature`/`max_tokens` built into `ChatRequest` then dropped; fixed 120s timeout; no `num_ctx` awareness; `stream: false`.
7. **Anything needing TLS.** `deepseek:` provider, `DiscordChannel` (default base `https://api.discord.com` — dead on arrival), any GitHub/GitLab tool. One missing feature blocks three shipped-looking ones.
8. **Ops.** No systemd unit, no entrypoint wiring, no retention/GC, no boot recovery (`runs_by_status` has exactly one caller), `panic = "abort"` in release.

## 3. Top 5 risks — what would embarrass Pii in real daily use

**R1 — A long conversation dies, ungracefully.** Every turn sends the full transcript; nothing compacts/truncates/counts tokens; `Limits::max_tokens` defaults to `u64::MAX` so the `budget` port can never catch it; `DEFAULT_TOML` declares no `[limits]`. Turn 25 of a real session returns a raw Ollama error, or the model silently ignores the beginning of the conversation. The #1 daily-use killer, no partial mitigation.

**R2 — Daemon restart is amnesia, and the run list lies afterwards.** Sessions vanish; new sessions reuse ids `s1`,`s2`…; runs left `Running` by a crash are never reconciled (nothing reads `runs_by_status(Running)` at boot) — `pii run list --status running` accumulates permanent zombies. `panic = "abort"` means one panicking node takes down every concurrent session. Timers survive (persisted `TimerWheel`) — which makes the sessions gap look even more like an oversight.

**R3 — Silent no-ops that make the system untrustworthy.** `memory = {enabled}` is decorative; `run.list` shows nothing; `memory.search` reports the vector lane ran when it has no data. Each minor; together they teach the operator the config surface is decorative. `reachability-check` should be pointed at config keys and event variants, not just functions.

**R4 — Blind waiting, and nothing stops.** No streaming: `granite4.2:3b` on CPU, four turns, tool calls — the TUI shows `you> ... [seq 0x1a]` for tens of seconds with no text. The whole delta pipeline exists (proto variant, bus routing, `subscribe.rs` filtering, TUI renderer); only the producer is missing. Ctrl-C / `run.cancel` cannot interrupt an in-flight model call or a running `bash`; a timed-out `bash` leaves orphaned grandchildren.

**R5 — The security model is airtight where the model has no leverage and open where it has all of it.** Pii's own code cannot dial an unapproved host. But an unscoped run gets `fs_read` over `/`, and the `bash` tool spawns children that inherit the daemon's full environment, constrained only by Landlock — which is **filesystem-only**. The model's shell can `curl` anywhere and read any secret in the daemon's env. Acceptable-ish for a single-user local box today; unacceptable the moment TLS lands and real API keys live in the daemon. **A hard prerequisite-ordering constraint, not a nice-to-have.**

## 4. Recommended next phase — "Phase 4: daily driver"

Ordered. Each item is spec-sized in the project's `specs/NN-*.md` idiom.

### P4.1 — Turn-loop hardening (streaming, compaction, cancellation, tool timeouts)
**Size**: L (split into 4.1a streaming+provider options, 4.1b compaction+cancel+timeouts). **Deps**: none. **Do this first.**
- **Streaming**: `stream: true` in `ollama.rs::build_request`, incremental NDJSON parse, emit `Event::AssistantDelta` through the existing `AgentNode::emit_live` path. Everything downstream already exists — highest value-per-line change in the repo.
- **Provider options**: pass `temperature`, `num_predict` (from `max_tokens`), `num_ctx` in the Ollama body. Today `temperature: 0.0` is a lie.
- **Compaction gate**: token estimate over `messages`; when it crosses a fraction of `num_ctx`, collapse oldest turns into one summary system message, write back to the session. Pair with a real default `[limits] max_tokens` in `DEFAULT_TOML`.
- **Cancellation**: `ctx.check()?` at the top of each turn and between tool calls; `run_child` polls the cancel token in its 10ms `try_wait` loop.
- **Process group + env scrubbing**: `Command::process_group(0)` (stable, no `unsafe`), kill the group on timeout/cancel; pass an explicit env allowlist rather than inheriting. Env scrubbing is a **prerequisite for P4.4**.
- **Per-tool `timeout()` / `concurrency_safe()`**: dsh item (e), already ranked #3 by the user.
- **Two more tools**: `grep` and `glob`/`ls`. Cheap; the difference between a 3B model succeeding and thrashing.

### P4.2 — Session durability and run-lifecycle truth
**Size**: M. **Deps**: none (new store table).
- Persist `Session` to redb; ULID or store-counter ids, never `len + 1`.
- Implement the `session.list` dispatch arm (the one daemon change spec 12 identified).
- Real `tokens_in`/`tokens_out` in `session.state`, accumulated from the `ChatResponse` counts the provider already parses.
- **Boot reconciliation**: at daemon start, sweep `runs_by_status(Running)` and mark each failed-interrupted (appending `NodeInterrupted`), or auto-resume behind an explicit policy flag.
- **Retention**: `pii admin prune --before <date>` plus an optional run TTL. The journal is append-only forever today; the rss-gate slope is the in-memory shadow of a disk problem nobody has hit yet.
- `pii tui --session <id>` to reattach.

### P4.3 — Global memory, v2 (rewrite spec 09 before implementing it)
**Size**: M. **Deps**: P4.1 (compaction moves the message-assembly seam), P4.2 (stable session ids for provenance).
Spec 09 has **three concrete defects** that will make the implementation silently useless:
1. **The dedup threshold cannot fire.** RRF scores are `Σ 1/(60 + rank)` (`rrf.rs:18`), so a doc ranked #1 in all three lanes scores ≈ `3/61` ≈ **0.049**, then × decay ≤ 1. Spec 09's "fused score exceeds 0.85 → skip the write" can *never* be true. Every re-flagged fact writes a new duplicate, forever. Dedup needs an absolute similarity signal — cosine over the trigram vector, or normalized-text `blake3` for the exact-repeat case — not the fused rank score.
2. **The vector lane has no data.** `memory_put` writes `vector: None`. The shared `write_fact` helper must compute and store the embedding, or the "BM25 + vector + graph" story is BM25 + graph with a false `lanes` report.
3. **The read path can't read.** `memory.search` returns ids only. Injecting "Relevant memory:\n- <doc text>" needs doc text in the hit payload or a new `memory.get`.

**On the MEMORIZE-marker write policy**: keep it as a *v1* choice, but not the *only* path — a 3B local model emits that marker unreliably and the failure is invisible. Ship it alongside an explicit human path: a `/remember <fact>` slash command in the TUI and a `pii memory add` CLI verb, both through the same `write_fact` helper. Store `session_id`, `run_id`, `created_ms` on each `Doc` for provenance/audit/revoke.

**Should sessions be persisted?** Yes (P4.2), keeping the two layers distinct: session transcript = verbatim, bounded, per-conversation; memory = distilled, cross-conversation, human-auditable.

### P4.4 — TLS in `pii-net` (outbound only)
**Size**: M–L. **Deps**: P4.1's env scrubbing (hard). **Verdict: yes, build it — but fourth, not first.**
- **Outbound only**: add `Scheme::Https` to `http_codec.rs:26-36`; wrap the `TcpStream` `dial.rs` returns in a rustls client connection. The authorize → pin → grant → dial chokepoint is unchanged; `net-check` keeps passing.
- **rustls + webpki-roots**, server-cert verification on, SNI from the parsed host, no client certs, no custom verifier. Prefer the `ring` provider over `aws-lc-rs` for build simplicity under `opt-level="z"` + fat LTO + `panic=abort`; verify the release profile still links.
- **No TLS on the listener.** The daemon is UDS + peercred; the TCP listener stays plaintext-loopback. Do not invent a localhost PKI.
- Add a `deps-check` invariant that rustls may appear only under `pii-net`, mirroring the redb-confined-to-`pii-store` rule.
- **Secrets**: decide storage (`[core] secrets` in config at 0600 vs env vs keyring shim) *in this spec*; make redaction the default everywhere.

### P4.5 — Make it a service
**Size**: S. **Deps**: none. **Do it opportunistically, any time.**
systemd user unit (or the container `entrypoint.sh` hook), `pii daemon --foreground` semantics for a supervisor, log rotation, meaningful exit codes from `pii sys status` for a healthcheck. The difference between "survives a file-level recreate" and "survives a rebuild"; a day of work.

### P4.6 — `pii web` read-only viewer (spec 12)
**Size**: M. **Deps**: P4.2. **Build it, but after P4.2, not now.**
Worth it for *observability*, not a second chat UI. The architecture in the draft is **right** (redb single-connection → proxy the daemon's UDS JSON-RPC over loopback HTTP; separate `pii-web` crate keeping axum out of core). Two amendments: sequence after P4.2 (a run browser on top of a `run.list` that returns `[]` and a missing `session.list` ships a viewer whose main job is displaying incorrect state); prefer **SSE over polling** for the live view (maps onto `event.subscribe` almost one-to-one, and makes P4.1's streaming deltas visible in the browser).

### P4.7 — Self-improvement: the honest next step
**Size**: M. **Deps**: P4.2 (durable evidence). **Explicitly *not* PR-opening.**
The stated end goal — the heal module opens a PR — is three new risk surfaces (TLS, a write-scoped credential, `fs`/network scope beyond the worktree) for the convenience of not typing `git push`. The interesting, dangerous, valuable part of self-healing is everything *before* the push, and Slice 1 already proved it works end-to-end.

**Slice 2 (now)**: graduate the proven loop into durable, reviewable output without the network.
- Commit the verified fix on a branch in the worktree; persist the patch as an artifact keyed by the heal signature.
- Extend the heal ledger from "attempt happened" to "attempt → patch → gate outcome → operator decision".
- Close the recorded Slice-1 follow-up: persist `scope_root` in the run header so `spawn_resume` honours it (`runs.rs:111` rebuilds workspace-scoped caps today; safe only because nothing writes after heal-fix's gate — a footgun waiting for Slice 2's first post-gate write).
- Add the `max_model_calls` budget dimension the opus review recorded as missing.

**Slice 3 (post-P4.4)**: `git push` + PR. Only then.

**The honest observation**: the strongest self-improvement machinery in this repo is not the heal module — it is `reachability-check` and `doc-drift-check`, deterministic no-model gates that convert regressions into machine-checkable facts. The highest-leverage self-improvement work is **extending those gates to the silent-no-op class** (a declared TOML config key with no reader; an event variant with no producer; an RPC method with no dispatch arm). That would have caught `memory = {enabled}`, `AssistantDelta`, and `session.list` automatically, and it costs a day.

### Ordering summary
```
P4.1 turn-loop hardening ──┬─→ P4.4 TLS (needs env scrubbing) ──→ heal Slice 3
                           │
P4.2 session durability ───┼─→ P4.3 memory v2
                           └─→ P4.6 web viewer
P4.5 service packaging  (independent, any time)
P4.7 heal Slice 2 + gate extension (independent, high leverage)
```

## 5. What to explicitly NOT build — and what to cut

**Do not build:**
- **The WASM / dynamic plugin layer** (dsh item (a)). Pii ships one static binary; adding a node kind is a `reg.register()` line and a rebuild. Dynamic loading buys reversible-effect plugin metaphysics that only pay off in dsh's Node.js multi-tenant world, and costs a wasmtime dependency plus a new capability story for guest code. The `NodeFactory` seam already exists — leave it as a compile-time seam.
- **Cross-process run/session write leases** (dsh item (d)). One daemon. Document the limitation; revisit if a second ever exists.
- **Nano-model Tier-0 routing** (pass 3). Not yet — you cannot justify a router without per-turn latency and token telemetry, and P4.2 is what produces it. Build the measurement, then decide.
- **Query rewriting, importance scoring, or auto-tuned thresholds in memory v1.** Spec 09's guardrails already say this; they are correct.
- **TLS on the daemon's own listener.** UDS + peercred is the right local security model.
- **Multi-user / multi-tenant anything.**

**Cut or fix (things that currently lie):**
- `OllamaEmbedder` — unreachable twice over. Either fix it (report real dims after a probe; take the caller's `Ctx` instead of minting `Ctx::root(CapSet::empty())`) or delete it and be honest that the vector lane is a trigram-hash lexical lane, not a semantic one.
- `BudgetKind::MicroUsd` — charge it from provider cost metadata or delete the dimension.
- `DiscordChannel` — feature-gate it off until TLS lands; its own default base is `https://`, which cannot work.
- The `bash` deny-list — keep it if you like, but stop describing it as a control. The controls are Landlock, `proc_argv0`, and `PathScope`.
- **Do not relax the file-size cap, the lint set, or the dependency gates to move faster.** They are why this codebase is coherent.

## 6. Open questions for the user

1. **Thread-cap defaults** (still open). My read of the measurement — uncapped slope 10620 < capped 11548, threads flat at 33 under a 3000-run soak — is that the caps are a debugging artifact that outlived its cause. Recommend raising defaults to 8/32 and keeping the env overrides. Yes/no?
2. **`panic = "abort"` in release.** Correct today (an unwind through `thread::scope` into the scheduler is worse). But one panicking node destroys every concurrent session. Keep abort until P4.2 makes sessions durable, then revisit with per-node `catch_unwind`? Or accept abort permanently?
3. **What is Pii's primary product — the terminal coding agent, or the assistant daemon?** If coding agent: P4.1 dominates and memory can slip. If assistant: P4.3 memory moves up. The current tree hedges — `pii -p`'s `DEFAULT_TOML` grants all five tools including `bash`, while `assistant.toml` grants only `fs_read` with `max_turns = 4`. Two different products, the inconsistency currently invisible.
4. **How much filesystem should an unscoped agent read?** `fs_read` is `/` today. Propose narrowing to workspace + an explicit allowlist. Any workflow that depends on reading outside the workspace?
5. **Secret storage for the TLS era**: config file at 0600, environment, or a keyring shim? Decide before P4.4 starts.
6. **Retention**: how long should runs and journals live? Forever is the only truly unbounded resource in the system.
7. **Does anything outside this repo hold session ids?** If pidag/scripts/notes reference `s1`/`s2`, the P4.2 id change needs a migration note.

---

### One-paragraph honest summary
Pii's engine is better than its product, and the gap is narrower than it looks: most of what's missing is a wire between two things that already exist. Streaming has a producer-shaped hole in a pipeline that is otherwise complete. Memory has a substrate with an eval harness proving it works and a production write path that omits the vectors. Sessions have a full transcript model that is never written to a store that is sitting right there. The next phase is not "build more" — it is roughly two weeks of connecting, persisting, and deleting the things that claim to work and don't. Do that, and Pii goes from "MVP-complete" to something you would actually trust on a Tuesday.
