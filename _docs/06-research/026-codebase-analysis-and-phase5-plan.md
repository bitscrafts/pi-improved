<!-- MERGED-CORPUS 2026-09-28T20:00:57Z: this file is the canonical document for its subject.
     It absorbed 2 document(s) which are now sections below, kept verbatim.
     Why: earlier review passes over fewer crates; their surviving findings are preserved as sections here.
     Originals archived at _docs/99-archive/06-research-merged-2026-09-28/ (nothing deleted). -->

# Pii — codebase analysis, debt/risk & Phase 5 plan (with the earlier review passes)

**Date:** 2026-09-15 · **HEAD:** `533d833` (branch `master`) · **Author:** Claude Opus 5 (orchestrator, resumed session)
**Status:** analysis document only — **no spec authored or edited, nothing committed, no gate/pipeline run beyond read-only verification.**

Structure: **Part 1 — Review** (codebase & package map) · **Part 2 — Findings** (technical debt & risk) · **Part 3 — Next-phase plan** · **Appendix — Assumptions & method.**

Every load-bearing number in this document was produced mechanically in this session on
the working tree at HEAD (commands in the Appendix). Where a claim comes from a prior
review document rather than this session, the document and line are cited explicitly.

---

## Part 1 — Review: codebase & package map

### 1.1 Workspace at a glance

| Metric | Value | Source |
|---|---|---|
| Workspace members | 18 `crates/pii-*` + `xtask` = 19 | `Cargo.toml:3-23` |
| Rust edition / MSRV | 2024 / 1.95 | `Cargo.toml:27-28` |
| Source lines (`*/src/*.rs`) | 35,398 (31,406 in `crates/`, 3,992 in `xtask/`) | `wc -l` this session |
| Integration-test lines (`*/tests/*.rs`) | 6,334 across 33 files | `find … -path '*/tests/*.rs'` |
| Tests passing at HEAD | 401/401 unit+integration; **doctest step fails** (see F-01) | `_tmp/orch-analysis-20260915/quality-gate.log` |
| Lint policy | `unsafe_code=forbid`, `missing_docs=deny`, `unreachable_pub=deny`, clippy `pedantic`+`nursery`+`cargo` warn, `unwrap/expect/panic/indexing_slicing/todo/unimplemented/print_*` deny | `Cargo.toml:60-100` |
| Release profile | `opt-level="z"`, fat LTO, `panic="abort"`, stripped, overflow-checks on | `Cargo.toml:106-112` |
| Binaries produced | `pii` (from `pii-cli`), `pii-mcp`, `pii-web`, `pii-evolve`, `xtask` | each crate's `src/main.rs` / `[[bin]]` |
| Feature flags that exist | `pii-net/testing`, `pii-proto/client`, `pii-server/testkit` (all three) | crate `Cargo.toml` `[features]` |
| Untracked large state | `.pidag/` 13 GB (9.8 GB `pidag.redb`, mtime today 22:25), `_tmp/` 15 GB, `target/` 58 GB — all gitignored | `du -sh`, `.gitignore:1-4` |

### 1.2 Crate dependency graph (intra-workspace edges only, from each crate's `[dependencies]`)

```
                 pii-abi ──────────────────────────────────────────────┐
                    │                                                  │
                 pii-core ─────────┬──────────┬───────────┬────────┐   │
                    │              │          │           │        │   │
                 pii-store      pii-net   pii-worker   pii-uds  pii-channels
                 │       │         │                      │
             pii-graph  pii-memory pii-model ─────────────┤
                 │         │          │                   │
                 │         │       pii-tools ◄── pii-proto (standalone; feature `client` → tokio)
                 │         │          │               │  │
                 └─────────┴──────────┴─── pii-server ┘  ├── pii-tui
                                          │   │          ├── pii-web   (+abi, uds)
                                          │   └── pii-mcp (+abi, core, tools, proto)
                                          │              └── pii-evolve (+core)
                                       pii-cli (+abi, core, store, graph, proto, uds, worker, tools, tui)

xtask → pii-proto, pii-store, pii-memory, pii-core
```

Verified layering invariants (this session, by reading `Cargo.toml` edges):

- `pii-abi` has **no** intra-workspace deps; `pii-proto` has **none** either (it re-declares wire types on purpose, `pii-proto/src/lib.rs:1`).
- `pii-store` is the only crate depending on `redb` (`pii-store/src/lib.rs:1` — "THE ONLY crate that knows redb"); `pii-graph`, `pii-memory`, `xtask` reach redb only through it.
- No surface crate (`pii-tui`, `pii-web`, `pii-evolve`) depends on `pii-server` or `pii-core` — except `pii-evolve → pii-core` (used for `Ctx`-free helpers; `pii-evolve/src/lib.rs:1` describes itself as "a thin CLIENT").
- **`pii-mcp` and `pii-cli` embed `pii-server` in-process** — the server is a library, `pii-cli::cmd_daemon` is the daemon entry point (`pii-cli/src/cmd_daemon.rs:1`).
- `pii-tools → pii-model` and `pii-tools → pii-uds`: the tool crate dials the daemon's UDS for `memory_search`/`memory_put` (`pii-tools/src/memory.rs:1`) and needs the provider seam for tool-schema types. This is the one "sideways" edge in an otherwise strict DAG. `cargo xtask deps-check` (`xtask/src/deps.rs`, "four hard invariants") exits 0 at HEAD (run this session), so the edge is sanctioned by the project's own gate.

### 1.3 Layer/crate responsibilities

| Layer | Crate | src files / lines | Owns | Boundary rule (from the crate's own `//!` docs) |
|---|---|---|---|---|
| ABI | `pii-abi` | 9 / 1,223 | `CapKind`, `Effects`, `PiiError`, `FailoverReason`+`classify`, `RunEvent`/journal types, `NodeInput`/`Outcome`, HTTP data types, id newtypes | "traits and types shared by every later Pii crate" (`lib.rs:1`); serde/schemars only |
| Kernel | `pii-core` | 12 / 2,175 | `Ctx` (enforcing capability context), `CapSet`+`PathScope`, `Budget`, `CancelToken`, injectable `Clock`, `HttpTransport` seam, `Observer`, `ChildProc`, fs cap gate | `clock.rs:1` "the ONLY `SystemTime::now` call site"; `net.rs:1` transport seam lives here so pii-net is swappable |
| Persistence | `pii-store` | 9 / 2,644 | redb tables (run/journal/snapshot, `mem_*`, timer wheel, approval, channel cursor, heal ledger, session) | `lib.rs:1` only redb crate; `mem.rs` second redb file |
| Engine | `pii-graph` | 17 / 4,900 | spec model, TOML loader, tiny expr language, node registry + 5 built-ins, scheduler (`sched/{state,dispatch,loop_helpers}`), journal/resume, validator, cron parser, ASCII render | generic over `GraphNode`/`NodeFactory`; parallelism = `std::thread::scope` (no tokio) |
| Wire | `pii-proto` | 7 / 1,456 | JSON-RPC `Method` (closed vocabulary), `Event`, `RpcError`, `run.*`/`session.*`/`sys.*` params, feature-gated `client` | `lib.rs:1` "the wire contract between clients and the daemon"; no core/tokio unless `client` |
| Process | `pii-worker`, `pii-uds` | 5 / 885; 1 / 120 | supervisor, NDJSON frames, hot reload, control plane; client-side UDS helpers | worker wire is decoupled from proto (`pii-worker/src/lib.rs:1`) |
| Net | `pii-net` | 11 / 2,223 | `NetService` egress chokepoint, grants/leases, authorize, dial (only `TcpStream::connect`), resolve (only `ToSocketAddrs`), hand-written HTTP/1.1 codec, listeners, TLS, test fakes | `lib.rs:1` "the one socket"; `dial.rs:1`, `resolve.rs:1` uniqueness claims |
| Model | `pii-model` | 4 / 1,813 | sync `ModelProvider` seam; `OllamaProvider` (over `HttpTransport`), `LlamaCppProvider` (in-process, `llama-cpp-2`), tool-schema helpers, scripted provider | `llama-cpp-2` is an unconditional dep (`Cargo.toml:41`, `pii-model/Cargo.toml`) |
| Memory | `pii-memory` | 12 / 1,140 | BM25 + i8 vectors + graph lane fused by RRF, decay, consolidation, embedders | "Zero" in-RAM index (`lib.rs:1`); redb via pii-store |
| Server | `pii-server` | 27 / 7,080 | `Engine`/`Session`/`RunHandle` (`lib.rs`), run registry bridge (`runs.rs`), method dispatch, conn/hello gate, UDS+loopback transports, subscribe, compaction, session durability, node kinds (`agent`, `tool`, `gate`, `heal_detect`, `memory`, `worker`, `spawn`, `debug_crash`), `daemon/*` (lifecycle, config, timer scheduler, approvals, channel router, status, daemon-only methods) | three concerns fused: engine-bridge + server nodes + app lifecycle |
| Tools | `pii-tools` | 7 / 1,396 | built-in tool set: bash (mediated), fs, grep/glob, memory (over UDS), sandbox (Landlock), worktree | all fs via `Ctx::fs()` (`fs.rs:1`) |
| Channels | `pii-channels` | 3 / 343 | `CliChannel` (NDJSON files), `DiscordChannel` (HTTP) | "inbound polling + outbound reply, no sockets" (`lib.rs:1`) |
| Surfaces | `pii-cli` (13 / 2,325), `pii-tui` (1 / 679), `pii-web` (1 / 297), `pii-mcp` (1 / 350), `pii-evolve` (4 / 358) | CLI subcommands; TUI protocol client; HTTP→UDS read-only viewer; MCP server+client bridge; boss-watch launcher | TUI/web/evolve are pure proto/UDS clients; cli and mcp embed the server |
| Dev tools | `xtask` | 16 / 3,992 | `deps-check`, `lint-files` (450-effective-line gate), `bench`, `rss-gate`, `net-check`, `memory-eval`, `proto-gen`, `doc-drift-check`, `reachability-check`, `silent-noop-check`, `heal-{seed,collect,prune}`, `promote-build`, `ci` | `main.rs:1` |

### 1.4 File-level map (every `src/*.rs`, physical lines, responsibility from its `//!` header)

Files ≥ 450 physical lines are marked **▲**; files the project's own gate flags are marked **⚠** (warn band 380–450 effective) / **✖** (error > 450 effective). Gate output this session: `cargo xtask lint-files`.

**pii-abi/src** — `lib.rs` 26 · `cap.rs` 56 capability bits · `effects.rs` 83 effect classes for parallel scheduling · `error.rs` 122 `PiiError` · `failover.rs` 448 typed `FailoverReason` + pure `classify()` (spec 37) · `http.rs` 117 egress data types · `ids.rs` 76 id newtypes · `journal.rs` 161 event-sourced journal types · `node.rs` 134 node I/O contract.

**pii-core/src** — `lib.rs` 40 · `ctx.rs` ▲503 enforcing capability context · `childproc.rs` 429 child-process machinery · `cap.rs` 293 scopes + `CapSet` · `fscap.rs` 220 fs gate · `budget.rs` 212 carveable accounting · `observe.rs` 168 events/metrics/observer mux · `cancel.rs` 85 · `net.rs` 82 `HttpTransport` seam · `clock.rs` 74 · `principal.rs` 41 · `config.rs` 28.

**pii-store/src** — `lib.rs` ▲582 store + run/journal/snapshot tables · `heal.rs` ▲591 heal-attempt ledger · `mem.rs` ▲540 ⚠ `MemStore` (2nd redb file) · `session.rs` 268 durable session · `approval.rs` 218 · `channel.rs` 168 cursor · `timer.rs` 165 timer wheel · `tables.rs` 86 · `codec.rs` 26.

**pii-graph/src** — `lib.rs` 27 · `run.rs` ▲569 ⚠ store-backed run/resume + scheduler tick · `sched/state.rs` ▲532 run-state machine · `sched/dispatch.rs` ▲458 node dispatch/retry/cancel · `sched/loop_helpers.rs` 60 · `sched/mod.rs` 14 · `render.rs` ▲513 ASCII renderer · `validate.rs` ▲481 static validation V0–V9 · `toml_load.rs` 385 · `node.rs` 355 harness ABI + registry · `nodes.rs` 326 built-in kinds · `spec.rs` 300 · `journal.rs` 262 · `run_helpers.rs` 232 · `cron.rs` 223 · `expr.rs` 88 · `blackboard.rs` 75.

**pii-proto/src** — `lib.rs` 318 wire contract · `event.rs` 326 push vocabulary · `client.rs` 270 (feature `client`) · `method.rs` 196 closed method set · `sys.rs` 122 · `run.rs` 116 · `session.rs` 108.

**pii-worker/src** — `supervisor.rs` 348 · `control.rs` 312 invoke/death/restart/reload · `frame.rs` 120 NDJSON frames · `spec.rs` 93 · `lib.rs` 12. **pii-uds/src** — `lib.rs` 120.

**pii-net/src** — `service.rs` ▲675 ⚠ `NetService` chokepoint (leases, accounting, dechunk) · `http_codec.rs` 346 · `listen.rs` 324 · `testing.rs` 250 (feature) · `authorize.rs` 224 · `grant.rs` 199 · `dial.rs` 79 · `resolve.rs` 63 · `lib.rs` 49 · `limits.rs` 10 · `clock.rs` 4.

**pii-model/src** — `ollama.rs` ▲705 Ollama chat provider · `lib.rs` ▲499 provider seam + scripted provider · `llamacpp.rs` 408 in-process (spec 35) · `llamacpp_tools.rs` 201 (spec 36).

**pii-memory/src** — `doc.rs` 245 · `consolidate.rs` 188 · `search.rs` 130 RRF fusion · `graph_lane.rs` 102 · `bm25.rs` 91 · `embed.rs` 90 · `quant.rs` 82 · `decay.rs` 57 · `vector.rs` 55 · `tokenize.rs` 43 · `rrf.rs` 36 · `lib.rs` 21.

**pii-server/src** — `agent.rs` ▲1,144 ⚠ `agent` node: provider loop + tool execution + memorize (2.5× the physical limit) · `runs.rs` ▲528 run registry (`spawn_blocking(pii_graph::run)`) · `lib.rs` ▲479 `Engine`/`Session`/`RunHandle` · `dispatch.rs` 403 method dispatch · `subscribe.rs` 243 · `session_durability.rs` 253 · `conn.rs` 240 hello gate · `compaction.rs` 217 · `registry.rs` 202 · `bus.rs` 199 · `tool.rs` 104 · `uds.rs` 100 · `loopback.rs` 16 · **`daemon/`**: `config.rs` ▲459 (TOML, `deny_unknown_fields`) · `mod.rs` 422 lifecycle/timer/cron/approvals/router · `methods.rs` 337 · `status.rs` 168 · `scheduler.rs` 155 · `router.rs` 128 channel router · `approvals.rs` 77 · **`nodes/`**: `heal.rs` ▲490 `heal_detect` · `gate.rs` 305 · `spawn.rs` 142 · `worker.rs` 112 · `memory.rs` 84 · `crash.rs` 61 · `mod.rs` 12.

**pii-tools/src** — `search.rs` 353 grep/glob · `memory.rs` 248 · `fs.rs` 197 · `lib.rs` 173 · `sandbox.rs` 145 Landlock · `bash.rs` 144 · `worktree.rs` 136.

**pii-channels/src** — `discord.rs` 158 · `cli.rs` 135 · `lib.rs` 50.

**pii-cli/src** — `cmd_oneshot.rs` 401 `pii -p` · `cmd_daemon.rs` 341 · `main.rs` 232 · `cmd_trace.rs` 232 · `cmd_run.rs` 231 · `cmd_ctl.rs` 226 · `cmd_soak.rs` 171 · `cmd_worker_exec.rs` 136 · `cmd_tui.rs` 105 · `cmd_sys.rs` 92 · `cmd_sandbox_exec.rs` 73 · `cli_util.rs` 49 · `ndjson.rs` 36.

**pii-tui/src** — `lib.rs` ▲679 ⚠. **pii-web/src** — `main.rs` 297. **pii-mcp/src** — `main.rs` 350. **pii-evolve/src** — `state.rs` 141 · `driver.rs` 132 · `main.rs` 62 · `lib.rs` 23.

**xtask/src** — `silent_noop.rs` ▲701 ✖ (458 effective — the only hard gate violation) · `main.rs` ▲567 ⚠ · `deps.rs` ▲557 · `heal_collect.rs` 398 · `promote_build.rs` 224 · `heal_seed.rs` 212 · `reachability.rs` 210 · `memory_eval.rs` 195 · `rss_gate.rs` 167 · `lint_files.rs` 155 · `bench.rs` 140 · `doc_drift.rs` 138 · `net_check.rs` 116 · `proto_gen.rs` 102 · `heal_prune.rs` 84 · `lib.rs` 26.

### 1.5 Test inventory (integration tests, `crates/*/tests`)

| Crate | Files | Lines | Notes |
|---|---|---|---|
| pii-server | 15 | 2,751 | `approvals, boss_watch(693), cancellation_brake, channels, handshake, latency, memory_rpc, model_routing, peercred, reattach, scheduler, session, slice_e_live, slice_f_promotion, spawn_node` |
| pii-graph | 5 | 1,161 | `diamond, examples, journal, loops, resume` |
| pii-net | 2 | 517 | `egress, http_codec` |
| pii-memory | 3 | 456 | `consolidate, lanes, persistence` |
| pii-model | 2 | 454 | `ollama_http, real_granite` (real-model test) |
| pii-evolve, pii-worker, pii-tui, pii-store, pii-mcp | 2/1/1/1/1 | 291/284/207/143/65 | |
| **pii-abi, pii-core, pii-proto, pii-uds, pii-tools, pii-channels, pii-cli, pii-web, xtask** | **0** | — | unit tests only (inline `#[cfg(test)]`) or none — see F-20 |

### 1.6 Non-code assets and process surface

- **Graphs** `graphs/starter/`: `assistant, boss-watch, consolidate, crash, heal-fix, heal-watch, react, slow, soak, soak_cron, verify-fix` (11 TOML graphs; `heal-fix.toml` 5.0 KB is the largest).
- **Scripts**: `quality-gate.sh` (fmt → clippy → `cargo test --workspace`), `scripts/mvp-verify.sh`, `scripts/phase2/{selfchange,uds-vs-loopback}.sh`, `scripts/phase3/{crash,cron,discord-e2e,hot-reload,secret-scan,shutdown,worker-kill}.sh`. **No `scripts/phase4.7/`** directory exists.
- **Docs**: `_docs/01-overview/01-overview/001-pii-vision.md`; `_docs/02-guides/{feature-gating,pi-local-llamacpp,pidag-way-of-work,pii-mcp-bridge}.md`; `_docs/06-research/001…026`; `_docs/manuals/{technical,user}-manual.{tex,pdf}`; `_docs/{INDEX,TOC}.md`.
- **Specs** `specs/01…37` (+ `36-part1/part2`), read-only for the workhorse per `AGENTS.md:50-55`.
- **Process state** `.pidag/`: run JSON for specs 01–11, 13–17, 20, 22, 24, 28, 29; **none for 12, 18, 19, 21, 23, 25–27, 30–37**; `config.toml` sets `quality_gate_script = /projects/pi-improved/quality-gate.sh`, `max_iterations = 4`, workhorse `ollama-cloud/glm-5.3-flash:cloud`.
- **HANDOFF.md**: 138,936 bytes, 53 `##` sections, newest-first; uncommitted working-tree edit adds a "CURRENT STATE — 2026-09-15" header (45 lines) pointing at this document.

---

## Part 2 — Findings: technical debt and risk

Severity: **S1** blocks the SDD process or affects production behaviour · **S2** real defect or drift with bounded blast radius · **S3** hygiene/structural. Each finding cites its evidence; "(prior review)" marks items first raised in `_docs/06-research/010` / `011` and **re-verified in code this session**.

### 2.1 Harness and process (S1)

| ID | Finding | Evidence |
|---|---|---|
| **F-01** | **The quality gate is RED at HEAD.** fmt PASS, clippy PASS, 401/401 tests PASS, then `Doc-tests pii_abi` fails `E0463 can't find crate schemars/serde_json`; script exits 1. Because `.pidag/config.toml` names this script as `quality_gate_script`, **no `pidag sdd` run can pass at HEAD**. Root cause (per HANDOFF working-tree diff, lines 8-17): `cargo clippy --workspace --all-targets` builds a second `libschemars-*.rlib` with a different feature set; the subsequent unified `cargo test --workspace` doctest links a stale hash. HANDOFF previously labelled this a "flake" (HANDOFF.md:104-121 in the committed version). | `_tmp/orch-analysis-20260915/quality-gate.log` (tail: `exit=1`); `quality-gate.sh:20-36`; `.pidag/config.toml [sdd]` |
| **F-02** | **AGENTS.md describes a workflow the project no longer follows.** AGENTS.md:9-37 mandates `pidag sdd` with glm-5.3-flash and "up to 3 iterations". Reality: `.pidag/` has no run state for specs 12, 18–19, 21, 23, 25–27, 30–37; `config.toml` says `max_iterations = 4`; HANDOFF records specs 27, 30–37 implemented by sonnet/laguna via `_tmp/brief-*.md` (HANDOFF working-tree diff lines 25-27; committed HANDOFF "COST RULE 1" §, ~line 1465). AGENTS.md:41 still says "`crates/` — the Pii Rust workspace (when Phase 0 lands)". | `AGENTS.md:9-37,41`; `.pidag/*.json` listing |
| **F-03** | **`specs/` was edited in the working tree by a non-planner actor.** `git diff specs/33-slice-e-live.md` rewrites R5 and its exit-criterion line (`-bash scripts/phase4.7/slice-e2e.sh` → "NOT SHIPPED … replace with `cargo test -p pii-server --test slice_e_live`"). AGENTS.md:50-55 makes spec editing exclusive planner work and says to `git checkout` such edits. The edit is *factually correct* (the script does not exist — `ls scripts/` confirms) but is a process violation and an uncommitted spec change with no owner. **This session did not touch it** (guardrail). | `git diff --stat` → `specs/33-slice-e-live.md | 15 +++++++++------`; `AGENTS.md:50-55` |
| **F-04** | **Spec 33's R5 exit criterion is machine-unverifiable at the committed spec**: it invokes `scripts/phase4.7/slice-e2e.sh`, which does not exist; spec 33 was still marked DONE (`29625cf`). | committed `specs/33-slice-e-live.md:60-63,101`; `ls scripts/` (no `phase4.7/`) |
| **F-05** | **Spec numbers 35/36 were repurposed** (now in-process llama.cpp and tool calls). The originally-planned 35 (HTTP fetch tool through the pii-net chokepoint) and 36 (research graph writes memory) have **no tracking spec**. | `specs/35-in-process-llama-cpp.md`, `specs/36-*`; HANDOFF working-tree diff lines 27-30 |
| **F-06** | **HANDOFF.md is the de-facto SSOT but is 139 KB / 53 newest-first sections** with closed items still listed as open and policy reversals (COST RULE 1) recorded only implicitly. The uncommitted "CURRENT STATE" header is the right shape but sits on top of the full history rather than replacing it. | `wc -c HANDOFF.md`; `grep -c '^## '` |

### 2.2 SSOT drift — documentation vs. Cargo/code (S2)

| ID | Finding | Evidence |
|---|---|---|
| **F-07** | `feature-gating.md` tabulates 9 planned flags (`evolve, web, tui, channels, tls, memory, sandbox, github, provider-*`); **none exist**. The only features in the workspace are `pii-net/testing`, `pii-proto/client`, `pii-server/testkit`. Doc 011 A.14 flagged `pii-evolve` "documents itself as feature-gated and is not" — still true. | `_docs/02-guides/feature-gating.md:30-38`; `grep -A5 '^\[features\]' crates/*/Cargo.toml`; `011:84` |
| **F-08** | `pii-mcp-bridge.md:1,45,63` and doc `024:99-102` (C7) say `pii-mcp` is "feature-gated (default off)". It is an unconditional workspace member (`Cargo.toml:20`) producing a separate `pii-mcp` binary on every build. | as cited |
| **F-09** | `AGENTS.md:1-2` ("a single static Rust binary") vs. reality: four binaries (`pii`, `pii-mcp`, `pii-web`, `pii-evolve`) and every build compiles llama.cpp because `llama-cpp-2` is a non-optional dep of `pii-model` (`Cargo.toml:41`). | `crates/*/src/main.rs`; `crates/pii-model/Cargo.toml` |
| **F-10** | Stale references: `AGENTS.md:58` cites a `pii-hw` unsafe carve-out (crate never existed); `clippy.toml:19-22` tells contributors to "use `pii_obs::record!`" (no such crate — doc `011:66` A.10 "`pii-obs` never existed"); `_docs/INDEX.md:73` "- [ ] Implementation started"; `pii-graph/src/node.rs:64` "the four built-in kinds" while `nodes.rs` registers five. | as cited |

### 2.3 Confirmed live defects in production paths (S1/S2)

| ID | Sev | Defect | Evidence (re-read this session) |
|---|---|---|---|
| **D1** | S2 | **TLS connection-lease leak** (prior review 010 A4, "still open"): lease count is incremented (`*n += 1`, `service.rs:285`), then `dial_tls(stream, &target.host)?` (`service.rs:293`) returns early on failure — the `Err(e)` arm that decrements (`service.rs:305-311`) is only reached when `dial()` itself fails. | `crates/pii-net/src/service.rs:283-318`; `010:139` |
| **D2** | S1 | **Hard-coded production fs-cap scope.** Every channel-triggered run gets `fs_read`/`fs_write` = `/projects/pi-improved/_tmp/phase3` (literal, four occurrences). Any deployment outside this checkout path either denies all channel fs access or, worse, grants access to an unrelated directory that happens to exist. Not in a test module. | `crates/pii-server/src/daemon/router.rs:46-70` |
| **D3** | S2 | **O(n²) session persistence** (prior review 010 A9(a), still open): the whole `Session` is re-serialised on every prompt/append/status change; failures are swallowed (`unwrap_or_default()`, `let _ =`). | `crates/pii-server/src/session_durability.rs:39-45`; `crates/pii-store/src/session.rs:22`; `010:239` |
| **D4** | S2 | Blocking `poll_once` (→ blocking pii-net HTTP) executed directly on a tokio worker via `tokio::spawn`, not `spawn_blocking`. | `crates/pii-server/src/daemon/router.rs:30-34` |
| **D5** | S3 | Per-tool-call raw OS thread with a 5 ms busy-poll loop instead of a timed join. | `crates/pii-server/src/agent.rs:551-566` |
| **D6** | S3 | `session_runs: BTreeMap<String, Vec<String>>` grows unbounded for the daemon's lifetime. | `crates/pii-server/src/lib.rs:113,341-345` |
| **D7** | S3 | Registry lock held across the `kill()` syscall; lint suppressed with `significant_drop_tightening`. | `crates/pii-worker/src/control.rs:116-124` |

Prior-review items **010 A1, A2, A3, A5, A7** are fixed (verified last session at `ollama.rs:196-259`, `agent.rs:548-551`, `pii-net/src/service.rs:364-455`, `agent.rs:227`, `runs.rs:56,190`; not re-read this session — see Appendix A.3).

### 2.4 Structural debt (S2/S3)

| ID | Finding | Evidence |
|---|---|---|
| **F-11** | **Error type is string-typed.** 357 sites of `PiiError::{Internal,Invalid,Unavailable}(format!(..))`. Providers emit `"ollama: upstream status {status}"` as prose; HTTP status and origin are not carried structurally. **Direct consequence:** `pii_abi::failover::classify` (spec 37, A1) has **zero non-test callers** — the taxonomy exists but nothing can feed it. | `grep -rEo 'PiiError::(Internal\|Invalid\|Unavailable)\(format!' crates \| wc -l` → 357; `grep -rn 'classify(' crates` → only `failover.rs` tests |
| **F-12** | **Physical file size vs. the gate's metric.** 18 `src` files exceed 450 *physical* lines (largest: `agent.rs` 1,144; `xtask/silent_noop.rs` 701; `ollama.rs` 705; `pii-tui/lib.rs` 679; `service.rs` 675). The gate measures *effective* lines (comments/blanks/`cfg(test)` excluded), so only **1 file fails** (`xtask/src/silent_noop.rs` 458) and 6 sit in the 380–450 warn band (`agent.rs` 432, `pii-tui/lib.rs` 435, `service.rs` 424, `mem.rs` 416, `run.rs` 411, `xtask/main.rs` 411). The gate is technically holding; the debt is that big files are propped up by 22 `too_many_lines` allows and non-counted test bodies. 135 `#[allow]/#[expect]` attribute lines in total. | `cargo xtask lint-files` output; `grep -c` this session |
| **F-13** | `pii-server` fuses three concerns (engine bridge / server node kinds / daemon app) in one 7,080-line crate — the largest by 45 %. Planned-doc layering (001–003) had these separate. | `wc -l crates/pii-server/src/**`; `lib.rs:1` |
| **F-14** | `pii-store::heal` bakes product-specific fields (`worktree, branch, patch_hash, model, tokens_in/out, micro_usd`) into the generic persistence crate. | `crates/pii-store/src/heal.rs:66-116` |
| **F-15** | **Absolute checkout path baked into tests**: 40+ sites of `/projects/pi-improved/_tmp…` in `#[cfg(test)]` bodies across 12 crates (`ctx.rs:389-417`, `fscap.rs:173`, `pii-store/src/{lib,session,heal,timer,channel,approval}.rs`, `pii-model/src/lib.rs:405-406`, `agent.rs:1014-1108`, …). Sanctioned by AGENTS.md:60 ("Tests under `_tmp/`"), but it means the test suite cannot run from any other clone path or CI checkout. | `grep -rn '/projects/pi-improved' crates --include='*.rs'` |
| **F-16** | Sync-blocking substrate inside an async daemon: `pii-graph` runs on `std::thread::scope`, bridged through `spawn_blocking` (`runs.rs:1`), while channels/timers are tokio. This is a deliberate design (`pii-core` never depends on tokio) but D4 shows the seam is easy to get wrong. | `crates/pii-server/src/runs.rs:1`, `daemon/router.rs:30-34` |
| **F-17** | Still-open items from earlier opus reviews (not re-verified this session): snapshots written-never-read (F12 in 009/010 E), `max_steps` counts ticks not nodes (F20), `pii-mcp` client self-test hang (010 §D). | `010:404-472` (§E), `010:346` (§D) |

### 2.5 Test coverage blind spots (S2)

| ID | Gap | Evidence |
|---|---|---|
| **F-18** | `pii-proto` — the wire contract (`method/run/sys/session/client.rs`, 1,456 lines) has **no** integration tests and only inline unit tests; a schema regression would surface first in a client. | §1.5 |
| **F-19** | No tests at all (inline or integration) verified for: `pii-store/src/mem.rs` (540 lines), `pii-tools/src/{bash,fs,worktree}.rs`, `pii-channels/src/discord.rs`, `pii-web` handlers, 9 of 11 `pii-cli` `cmd_*.rs`. | prior-session Sonnet sweep, spot-checked (`grep -c 'cfg(test)'`) |
| **F-20** | Nine crates have zero `tests/` directories (§1.5). `sched/dispatch.rs` (458 lines) is covered only transitively through `pii-graph/tests`. | §1.5 |

### 2.6 Operational / environment risk (S3)

| ID | Finding | Evidence |
|---|---|---|
| **F-21** | **86 GB of untracked state on disk**: `target/` 58 GB, `_tmp/` 15 GB, `.pidag/` 13 GB (`pidag.redb` 9.8 GB, plus ~3 GB of `*.events.jsonl` up to 1.1 GB each, and 20 `pidag.redb.stale-*` backups). `pidag.redb` mtime is today 22:25, i.e. something still writes to it. All gitignored, so invisible to `git status`; a full disk would fail the gate in confusing ways. | `du -sh`; `ls -la .pidag/` |
| **F-22** | `real_granite.rs` (pii-model) is a real-model integration test in the default `cargo test --workspace` path; whether it self-skips without a model file was not verified this session. | `crates/pii-model/tests/real_granite.rs` |

### 2.7 Risk summary

| Risk | Likelihood | Impact | Driven by |
|---|---|---|---|
| Every "DONE" since the gate went red is self-reported, not machine-proved | certain (it is red now) | high | F-01, F-04 |
| Process documentation and actual executor diverge → next contributor/agent follows the wrong workflow | certain | medium | F-02, F-03, F-06 |
| Channel-triggered runs silently mis-scoped outside this checkout | high on any other host | high | D2 |
| Track A (failover) cannot progress without a design decision on the error type | certain | medium | F-11 |
| Large-file/allow debt compounds under the effective-line gate | medium | medium | F-12, F-13 |
| Disk exhaustion mid-run | medium | medium | F-21 |

---

## Part 3 — Next-phase plan (Phase 5)

**Recommendation: a short re-baseline sub-phase (5.0) before continuing plan-025 Track A.**
Rationale: (i) everything later validates through a gate that fails today; (ii) D1–D3 are ≤1 file each, production-affecting, and are the cheapest possible way to re-prove the SDD harness end-to-end; (iii) A2 requires a structured error type — a planner decision, not something a workhorse should infer from string parsing.

Conventions assumed for every step (from `AGENTS.md`, `_docs/02-guides/pidag-way-of-work.md`, and the `sdd-orchestrate` / `spec-author` skills): planner authors the spec with TDD Contract + machine-checkable Exit Criteria; workhorse implements; `quality-gate.sh` + validate script run per iteration; planner reviews the load-bearing diff and commits; HANDOFF updated same turn. **`specs/` is written only by the planner.**

### Phase 5.0 — Re-baseline (harness, live defects, SSOT)

Ordering: 5.0.1 first (blocking); 5.0.2–5.0.4 next, parallel-safe (disjoint crates), **each via the SDD harness**; 5.0.5 after (touches several `Cargo.toml`); 5.0.6–5.0.7 planner-only, can run concurrently with 5.0.2+.

| Step | Scope | Machine-checkable acceptance |
|---|---|---|
| **5.0.1 Gate literally green** (F-01) | Root-cause the `pii-abi` doctest E0463 (duplicate `libschemars` rlib from feature unification between the `--all-targets` clippy build and `cargo test`). Fix the cause (e.g. align feature sets, or run `cargo test --workspace --doc` as its own step with a fresh profile), or make the split invocation the script's SSOT. Update `.pidag/config.toml` if the script path/name changes. | `./quality-gate.sh; echo $?` → `0` three consecutive runs, one of them after `cargo clean`. No `"flake"` wording left for this item in HANDOFF's CURRENT STATE. |
| **5.0.2 D1 TLS lease** | Single exit path: any failure after the increment decrements (wrap dial+TLS in one `Result` before the match, or `Drop`-guard the lease). | New `pii-net` test (feature `testing`, fake TLS handshake failure) asserting `grant_info()` lease returns to 0; **test must be red on pre-fix code** (reviewer reverts and re-runs). |
| **5.0.3 D2 channel scope** | Channel fs scope comes from `daemon/config.rs` (default `<PII_HOME>/channels/<name>`), never a literal; also fix `resolve` of the scope at daemon start so a missing dir is a config error, not a silent deny. | `grep -rn '/projects/pi-improved' crates --include='*.rs' \| grep -vE 'cfg\(test\)\|tests/'` → 0 hits in non-test code; test: path outside the configured scope is `Denied`, inside is allowed. |
| **5.0.4 D3/D6 session durability** | Append-per-message keyed `(session_id, seq)` + periodic snapshot; surface persist errors through `Observer`; cap `session_runs` (config key, default e.g. 256 per session, LRU). | Counting-store test: bytes written over 200 appends ≤ 2× final session size; a persist failure emits an `ObsEvent`; `session_runs` length ≤ cap after cap+10 runs. |
| **5.0.5 First real feature flag** (F-07/F-09) | `pii-model/local-llm` (make `llama-cpp-2` optional) propagated to `pii-server`/`pii-cli`; default **on** only in `pii-cli`. Delete the `.cargo/config.toml` clang requirement for non-default builds if possible. | `cargo tree -p pii-server --no-default-features -e features \| grep -c llama-cpp` → 0; default `./quality-gate.sh` → 0; `cargo build -p pii-cli --no-default-features` succeeds. |
| **5.0.6 SSOT reconciliation** (planner; F-02, F-05, F-07–F-10) | Rewrite `AGENTS.md` to the *actual* implementer policy (see decision gate below); drop `pii-hw`, fix `crates/ … (when Phase 0 lands)`, `max_iterations` 3→4; `_docs/INDEX.md` status; `feature-gating.md` becomes a target-vs-current table; `pii-mcp-bridge.md` and doc 024 C7 corrected (separate ungated binary); `clippy.toml` `pii_obs` text → the real observer API; `node.rs:64` "four"→"five"; **decide** the orphaned original 35/36 content: author tracking specs 38 (HTTP-fetch tool via pii-net chokepoint) and 39 (research-writes-memory) or record an explicit cut; record the COST RULE 1 reversal. | New `scripts/spec-refs-check.sh` (~20 lines): every backticked path under a spec's *Exit Criteria* exists → exit 0 (fails today on spec 33); `grep -c 'Implementation started: \[ \]' _docs/INDEX.md` → 0; `grep -rn 'pii-hw\|pii_obs' AGENTS.md clippy.toml` → 0. |
| **5.0.7 Resolve spec-33 working-tree edit + HANDOFF hygiene** (planner; F-03, F-04, F-06) | Planner adopts the working-tree edit to spec 33 (it is correct) **as a planner-authored change** or reverts it and amends via a new spec addendum — either way the edit gets an owner and a commit. HANDOFF: pinned "CURRENT STATE" ≤ 80 lines + an open-ledger keyed by the IDs in this document (F-xx / D-x); archive history to `_docs/99-archive/handoff-2026-09.md`. | `git status --porcelain specs/` empty after the planner's commit; `wc -c HANDOFF.md` < 40,000; ledger section lists every D1–D7 / F-01–F-22 with a status. |
| **5.0.8 Disk hygiene** (operator; F-21) | Move (never delete) `pidag.redb.stale-*` and `*.events.jsonl` > 100 MB to `_tmp/_trash/pidag-$(date +%F)/`; `cargo clean -p` for crates not touched in 30 days is optional. Per the standing no-`rm -rf` rule, `mv` only. | `du -sh .pidag` < 3 GB; nothing deleted (trash dir exists and is listed in HANDOFF). |

**Harness decision gate (end of 5.0):** if 5.0.2–5.0.4 all landed through `pidag sdd` with the gate green, `AGENTS.md` keeps pidag as executor with the model chain stated truthfully. If two of three needed manual sonnet/laguna rescue, `AGENTS.md` is rewritten to the brief-driven process that is actually in use. Either outcome is acceptable; an untrue `AGENTS.md` is not.

### Phase 5.1 — Track A: failover becomes real (plan 025 §5, A2–A5)

> **Related (2026-09-21):** `039-jev-system-one-decision-layer-analysis.md` —
> supplies the *decision-engine* half that plan 025 leaves open (025 §4.4 item 5
> names a tier-0 classifier but does not say what provides it). Analysis only;
> it proposes a System-1 (Jev/Laya-class) decision source feeding the existing
> `route` node, with the graph staying the sole authority over edges. No
> implementation.

| Step | Scope | Acceptance |
|---|---|---|
| **A2 structured provider failure** (planner design decision first; F-11) | Add `pii_abi::ProviderFailure { status: Option<u16>, origin: Origin, provider: String, message: String }` carried by a new `PiiError::Provider(ProviderFailure)` variant (preferred over overloading `Unavailable`); `OllamaProvider`/`LlamaCppProvider` construct it; `classify` gains `impl From<&ProviderFailure>` (or a `classify_provider` adapter). **No message-string parsing anywhere in `pii-server`.** | `grep -rn 'upstream status {' crates/pii-model` → 0; tests: Ollama 429 with aggregator origin → `UpstreamRateLimit`/`NextProvider`; llama.cpp `n_ctx` overflow → `ContextOverflow`/`Compress`; `grep -rn 'classify' crates --include='*.rs' \| grep -v failover.rs \| grep -v '#\[cfg(test)\]'` ≥ 1. |
| **A3 failure-as-payload** | Model/agent node emits `Outcome` on a `fail` port with `{status, reason, provider}`; non-provider errors remain `Err`. Lands *before* the `agent.rs` split (5.2.1) so the split does not churn the file A3 edits. | Graph test: `ScriptedProvider` failing once with `ContextOverflow` routes to `fail`; journal shows the payload. |
| **A4 failover ladder graph** | `graphs/starter/failover.toml`: sequential model nodes, `route` on `$.payload.reason`, compress branch, `retry` for `RetrySame`. | `pii graph validate graphs/starter/failover.toml` → 0; run test: chain advances on `NextProvider`, never on `Compress`. |
| **A5 operator surface** | Reason visible in `pii run show`; journal signature documented in research doc 006. | CLI test asserting the reason string; doc updated. |

### Phase 5.2 — Structural debt ratchet (interleave after A3; separate worker, disjoint files)

1. Split `agent.rs` → `agent/{mod,turn,tools,memorize}.rs`; `service.rs` → lease/accounting vs dechunk; no behaviour change. Acceptance: test count identical (401 + additions), `too_many_lines` allows 22 → ≤ 4, `lint-files` warn list shrinks by ≥ 2.
2. D4 `spawn_blocking` for channel polls; D5 `recv_timeout` join; D7 drop the lock before `kill()`; bounded accept pool at `listen.rs:190`.
3. Tests where blind (F-18–F-20): `pii-proto` round-trip for every `Method`/`Event`; `pii-store` `MemStore` contract; `pii-tools` bash/fs/worktree; `pii-web` handlers.
4. `heal.rs` (F-14): product-specific fields move into a server-owned `extra: serde_json::Value`; store schema stays generic.
5. F-15: replace `/projects/pi-improved/_tmp` in tests with `concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp")` or a `pii_core::test_tmp()` helper. Acceptance: `grep -rn '/projects/pi-improved' crates` → 0.
6. **Ratchet rule**: `xtask lint-files` gets a checked-in baseline file; new violations fail; target **0 effective violations** (fix `silent_noop.rs`) and the 6 warn-band files back under 380 effective.

### Phase 5.3 — Track B routing (plan 025 B1–B3) + old review leftovers

B1 deterministic escalation ladder (no LLM judge), B2 safety-rule ordering in config, B3 peak-aware route; plus F-17 items (snapshots: use in resume or delete; `max_steps` semantics; `pii-mcp` client hang). B4 (tier-0 local router) stays research until B1–B3 are in production.

### Sequencing rationale

- 5.0.1 before anything: a red gate makes every "DONE" self-reported — the exact failure mode the review passes (009, 010, 011) kept catching.
- D1–D3 before A2: production-affecting, ≤ 1 file each, and they double as the cheapest harness re-validation.
- A2 as an explicit planner step: the string-typed `PiiError` is the root of both the failover gap (`classify` has no callers) and the 357-site string-error tax; decide the shape once.
- Debt ratchet after A3, not before: splitting `agent.rs` first would churn the file A3 must edit.

**Two planner decisions are prerequisites for spec authoring:** (1) the `PiiError` extension shape for A2; (2) re-queue-or-cut for the orphaned original 35/36 content. Recommendation: re-queue the HTTP-fetch tool as spec 38 (it closes the "one socket" invariant for tools) and cut research-writes-memory until Track A lands.

---

## Appendix — Assumptions, method, and guardrail compliance

### A.1 Explicit assumptions

1. **HEAD `533d833` is the analysed state**; the two uncommitted working-tree edits (`HANDOFF.md`, `specs/33-slice-e-live.md`) were read but treated as *unowned* changes, not as part of HEAD.
2. **The gate log from 12:01 today is representative of HEAD** (`_tmp/orch-analysis-20260915/quality-gate.log`). The gate was **not re-run** this session (it compiles llama.cpp, ~1 min wall / 3.5 min CPU); no commits have landed since the log was produced (`git log` shows HEAD unchanged).
3. **"Production path" vs "test"** was decided by whether a site sits inside `#[cfg(test)]`/`tests/`. D2 was read in full to confirm it is production code.
4. **Prior-review "fixed" claims (010 A1/A2/A3/A5/A7)** and the test-blind-spot list (F-19) are carried from last session's verification and were only spot-checked, not fully re-read, this session.
5. **`.pidag/` run-state absence** is taken as evidence a spec did not go through `pidag sdd`; a run executed from a different project root would leave no trace here — considered unlikely given `config.toml` `root`.
6. Model/executor names (sonnet, laguna, glm-5.3-flash) are quoted from HANDOFF/AGENTS as written; this session did not verify which model produced which commit.

### A.2 Method (all read-only)

`git status/diff` · `wc -l` over every `src`/`tests` `.rs` · `grep` of `[dependencies]`/`[features]` in every `Cargo.toml` · `//!` header extraction per file · `cargo run -p xtask -- lint-files` and `-- deps-check` (compile xtask only; both read-only) · `grep -rEo 'PiiError::(Internal|Invalid|Unavailable)\(format!'` → 357 · `grep -rn 'classify('` · `grep -rEn '#\[(allow|expect)\('` → 135 · `grep -rn '/projects/pi-improved'` · direct reads of `router.rs:40-72`, `service.rs:283-318`, `node.rs:62-66`, `quality-gate.sh`, `.pidag/config.toml`, `.gitignore` · `du -sh` · citation look-ups in research docs 010, 011, 024, 025 and `_docs/02-guides/*`.

### A.3 Guardrail compliance

- `specs/` **not written or edited** by this session (the pre-existing working-tree diff on spec 33 is reported as F-03 and left untouched).
- All work foreground; no background agents, cron, loops, Monitor, or scheduled wake-ups.
- Nothing committed, no gate/pipeline/pidag run triggered; only `cargo run -p xtask -- lint-files` (read-only analysis tool).
- No files deleted; the previous draft of this document was copied to `_tmp/orch-analysis-20260915/026-draft-prev-*.md` before being replaced.
- Files written: this document only.


---

## [merged] Pii (`pi-improved`) — full-codebase adversarial review, pass 1

<!-- Absorbed from `026-codebase-analysis-and-phase5-plan.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

**Date**: 2026-09-08 · **Reviewer**: claude-opus-5 · **Target**: `/projects/pi-improved` @ `24f6427`
**Scope**: 16 crates, Phase 4 (4.1a → 4.7 partial). Every claim below cites `path:line`
and was checked against the code in this tree — several were checked against a **running
process** (marked PROVEN) rather than inferred.

**Verdict: NEEDS WORK.** The substrate is still the good part. But Phase 4.1b/4.1c
introduced three defects that, together, mean **the agent's tool loop does not function
against a real model provider**. Everything P4.7 wants to build — patch, validate,
escalate, promote — is a tool loop.

---

## A. Regressions from Phase 4

### A1 — CRITICAL (P4.1c): the agent can never execute a tool against a real provider

`AgentNode::run` now calls the streaming path:

- `crates/pii-server/src/agent.rs:193` — `self.provider.chat_streaming(ctx, &req, &mut on_delta)`
- `crates/pii-model/src/ollama.rs:228-233` — `OllamaProvider::chat_streaming` returns
  `ChatResponse { text, tool_calls: Vec::new(), ... }`. **`tool_calls` is hardcoded empty.**

Consequence: `resp.tool_calls.is_empty()` at `agent.rs:229` is *always* true, so every
model reply is treated as a final answer and the node returns on `ok` at turn 1.
`run_tool_calls` (`agent.rs:339`) is unreachable in production. `max_turns`, the tool
schemas sent in `ChatRequest.tools`, `files_changed`, the heal-fix agent's edit loop —
all dead.

Why no test catches it: the trait's **default** `chat_streaming`
(`crates/pii-model/src/lib.rs:132-141`) delegates to `chat`, which *does* return tool
calls. Every test provider (`RecordingProvider` `agent.rs:462`, `DeltaProvider`
`agent.rs:626`, `ScriptedProvider`) either uses that default or returns no tool calls.
Only the real `OllamaProvider` overrides it, and only the real one is broken.

Ollama does deliver tool calls in the stream — `OllamaMessage.tool_calls` is already
declared and `#[serde(default)]` (`ollama.rs:45-46`). The streaming accumulator simply
discards them.

**Fix**: accumulate `message.tool_calls` across NDJSON lines in `chat_streaming` the same
way `text` is accumulated (`ollama.rs:219-227`), and return them.

### A2 — CRITICAL (P4.1b): every tool that declares a `timeout()` runs with EMPTY capabilities

`crates/pii-server/src/agent.rs:412-417`:

```rust
let handle = std::thread::spawn(move || {
    tool.call(
        &Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited()),
        &args,
    )
});
```

The caller's `ctx` is **discarded** and replaced with an empty-capability root context.
`CapSet::empty()` is `Default` (`crates/pii-core/src/cap.rs:153-155`) — no kinds, no
path scopes. Therefore:

- `bash` (`crates/pii-tools/src/bash.rs:85` → `ctx.proc()?`) → `PiiError::Denied`, always.
- `grep` / `glob` (`crates/pii-tools/src/search.rs:164` → `ctx.fs()?`) → denied, always.

Those are exactly the three tools that declare a timeout (`bash.rs:47-51` 120 s,
`search.rs:139-142` and `search.rs:211-214` 30 s each). The untimed tools (`fs_read`,
`fs_write`, `edit`, `multi_edit`, `lib.rs:41-43` → `None`) take the inline branch at
`agent.rs:407-409` and keep the real ctx, so they still work — which is why the Slice-1
heal e2e (fix agent with fs/edit tools and **no bash**) passed and hid this.

`Ctx` is documented `Send + Sync` and cloneable (`crates/pii-core/src/ctx.rs:36`), so the
fix is one line: clone the caller's ctx into the worker thread. Do **not** substitute a
root capset — that would turn a confinement bug into an escalation (the heal run's
`scope_root` fs_write confinement lives in that ctx).

Why no test catches it: `slow_tool_times_out` (`agent.rs:566-615`) uses a fake `SlowTool`
whose `call` ignores `ctx` entirely.

### A3 — HIGH (P4.1c + P4.4): `request_streaming` is not byte-equivalent to `request`, and streaming emits zero deltas in production

`NetService::request` decodes the body through `parse_response`
(`crates/pii-net/src/service.rs:353`), which honours `Content-Length` **and** dechunks
`Transfer-Encoding: chunked` (`crates/pii-net/src/http_codec.rs:165-194`, `read_chunked`
at `196-233`).

`NetService::request_streaming` does neither. It parses only the head
(`service.rs:395`) and then treats every remaining raw byte as body, forwarding it
verbatim to `on_chunk` and returning it as `HttpResponse.body`
(`service.rs:397-418`). No dechunking. No `Content-Length` truncation. The
`Connection: close` header (`http_codec.rs:66`) is what makes read-to-EOF terminate at
all.

**PROVEN**: the local Ollama answers a `"stream": true` chat with

```
HTTP/1.1 200 OK
Content-Type: application/x-ndjson
Connection: close
Transfer-Encoding: chunked

93
{"model":"granite4.2:3b",...,"done":false}

90
{...}
```

(raw socket probe against `127.0.0.1:11434`, this session).

So the first line of every `on_chunk` payload is a hex chunk-size line. The Ollama delta
closure (`ollama.rs:192-203`) does:

```rust
for line in String::from_utf8_lossy(bytes).lines() {
    let Ok(l) = serde_json::from_str::<OllamaStreamLine>(line) else {
        return; // partial line: the next read completes it in this model
    };
```

`"93"` is not valid JSON → `return` → the closure exits before reaching the JSON payload
on the next line. **`on_delta` never fires. `Event::AssistantDelta` is never emitted
against a real Ollama.** P4.1c's headline feature is a silent no-op in production — the
exact pathology the Phase-4 plan was written to eliminate (`007-...-plan.md:16`).

The final `ChatResponse.text` is still correct: the accumulator at `ollama.rs:219-223`
`filter_map`s unparseable lines away, and the JSON objects happen to survive as intact
lines through the chunk framing. So the user gets the right answer with no live text.

Second, independent defect in the same closure: the `return` comment is wrong. There is
**no cross-chunk line buffer**. A TCP read that lands mid-JSON-line drops that line
permanently — the tail arrives at the head of the next chunk and doesn't parse either.
The R2 acceptance test's `ChunkedTransport` (`ollama.rs:333-377`) hands three *complete,
unframed* NDJSON lines, so it cannot detect either problem.

**Fix**: dechunk inside `request_streaming` (reuse `read_chunked`'s state machine
incrementally) so streaming and buffered bodies are byte-identical, and keep a residual
`String` buffer in the Ollama delta closure across chunks. Then make the acceptance test
feed *chunk-framed* bytes split at a mid-line boundary.

### A4 — HIGH (P4.4): connection-lease leak on TLS setup failure

`crates/pii-net/src/service.rs:276-286` increments the lease counter, then:

```rust
match dial(...) {
    Ok(stream) => {
        let stream = if scheme == Scheme::Https {
            Stream::Tls(Box::new(dial_tls(stream, &target.host)?))   // <- service.rs:292
        } ...
```

The `?` returns from `connect` **before** `Conn` is constructed, so `Drop for Conn`
(`service.rs:133-140`) cannot decrement, and the compensating decrement in the dial-error
branch (`service.rs:305-311`) is not on this path. Every `dial_tls` failure — a bad SNI
name (`dial.rs:68-69`) or a rustls client-config error (`dial.rs:70-71`) — permanently
burns one lease against `grant.max_conns` for that `(principal, key)`. After `max_conns`
such failures the principal can never reach that host again for the daemon's lifetime.

**Fix**: hoist the lease decrement into a guard, or build the `Conn` first and wrap the
stream after.

### A5 — TLS cert verification: correct, but untested on the real path

`dial_tls` (`crates/pii-net/src/dial.rs:54-73`) uses `ClientConfig::builder()
.with_root_certificates(webpki_roots)` `.with_no_client_auth()` — the **default** rustls
verifier, no `dangerous()` escape hatch, SNI from the parsed host. That is right.

Two caveats:
- The handshake is lazy (`StreamOwned`), so a certificate failure surfaces later as
  `PiiError::Unavailable("net: read: ...")` from `Conn::read` (`service.rs:184-185`) —
  indistinguishable from a transient network error, and therefore likely to be treated as
  retryable by callers. A real cert rejection should not be retried.
- No test performs a real handshake; the shipped tests are `parse_url_https`
  (`http_codec.rs:288`), `parse_url_https_default_port` (`:298`) and a dispatch test.
  Byte budgets on TLS charge *plaintext* bytes (post-decrypt) while plain HTTP charges
  wire bytes, and the handshake bytes are charged to nobody. Minor, but the accounting is
  not scheme-neutral as R3 claims.

### A6 — MEDIUM (P4.1b): compaction never fires in the shape that matters

`compact` (`crates/pii-server/src/compaction.rs:57-88`) defines a "turn" as a
`Role::User` message (`:69-76`). Inside one agent run there is exactly **one** user
message; everything after it is Assistant/Tool pairs. So `n_turns` is 1, the guard
`n_turns <= keep_recent_turns` (default 6, `compaction.rs:26-29`) hits at
`compaction.rs:78-83`, and `compact` returns the input **unchanged**.

Result: a long tool loop (dozens of tool results, each up to 64 KiB) sails past the
threshold; `needs_compaction` keeps returning `true` at `agent.rs:159`, `compact` keeps
returning the same list, nothing ever shrinks, and the context overruns `num_ctx` anyway.
The one shape a coding agent actually produces is the one compaction cannot handle.

Secondary: if it *did* split there, an Assistant message carrying `tool_calls` can be
collapsed into the summary while its matching `Role::Tool` result survives in `recent` —
an orphaned tool result. Harmless against Ollama only because of A7.

### A7 — MEDIUM: tool-call structure is dropped on the wire

`message_value` (`crates/pii-model/src/ollama.rs:237-245`) serialises **only** `role` and
`content`. `ChatMessage.tool_calls` and `tool_call_id` (`lib.rs:47-52`) are never sent.
So after a tool round-trip the model sees a `role: "tool"` message with no linkage and no
record that it ever requested the call (a tool-call turn usually has empty `content`).
Multi-step tool use degrades to the model re-requesting the same tool. This becomes
load-bearing the moment A1 is fixed.

### A8 — env scrub: works, and is a known blocker for the GitHub slice

`CHILD_ENV_ALLOWLIST` (`crates/pii-core/src/childproc.rs:197`) keeps
`PATH/HOME/LANG/LC_ALL/TERM/PWD/TMPDIR` plus every `PII_*`, applied at both `Command`
sites (`childproc.rs:42` and `childproc.rs:258`). `bash` works: `/bin/sh`, `cargo` and
`git` all resolve from `PATH` + `HOME`. Secrets are genuinely dropped
(`run_child_scrubs_environment`, `childproc.rs:347`).

Two notes:
- **Forward blocker**: `SSH_AUTH_SOCK`, `GIT_*`, `GH_TOKEN`, `SSL_CERT_FILE`, `CARGO_*`,
  `RUSTUP_*` are all dropped. That is correct today and is exactly right for a model's
  shell. It also means the P4.7 GitHub slice cannot `git push` or run `gh` from the bash
  tool. Decide the credential path now: inject per-call via the existing `extra` env pairs
  on `spawn_child` (`childproc.rs:176`), scoped to a dedicated node kind — **do not widen
  the allowlist**.
- Cosmetic dead code: `ChildProc::spawn` applies `extra` twice, via
  `scrubbed_env(&live_env(), env)` (`childproc.rs:42`) and again in the loop at
  `childproc.rs:49-51`.

### A9 — session durability (P4.2): sound, with two real growth/atomicity issues

Verified correct:
- ids come from a durable monotonic counter — `Engine::create_session`
  (`crates/pii-server/src/lib.rs:247`) → `Store::next_session_id`
  (`crates/pii-store/src/session.rs:113`). No `len + 1` collision.
- boot re-seed (`session_durability.rs:146-158`) and `session.list`
  (`session_durability.rs:47-57`) are consistent: the list reads the RAM map, which the
  boot seed fills.
- `reconcile_boot` (`session_durability.rs:165-184`) genuinely clears zombies: it sets
  `header.status = Failed` then calls `append_event`, and `append_events` repoints
  `RUN_BY_STATUS` from `header.status` in the same transaction
  (`crates/pii-store/src/lib.rs:266-282`). No stale index.
- no redb second-opener risk: one `Store`, one process.

Two defects:
- **(a) O(n²) writes per session.** `persist_session` (`session_durability.rs:38-44`)
  serialises the *entire* `Session` — including the full transcript — and it is called on
  every assistant turn (`session_durability.rs:129`). A 200-turn boss session rewrites the
  whole transcript 200 times. Combined with the append-only journal this is the second
  unbounded write amplifier.
- **(b) lost token counts.** `record_assistant` does a non-atomic read-modify-write of the
  session token totals (`session_durability.rs:95-105`). Two runs finishing concurrently
  in one session lose one update. `session.state` then under-reports, which is the exact
  class of lie P4.2 was built to remove.

### A10 — `spawn_resume` restores `scope_root` but does not re-validate it

`runs.rs:138` recovers the scope from the journal; `spawn_run` validates
(`runs.rs:42-44`) but `spawn_resume` does not call `validate_scope_root`
(`runs.rs:237-259`). Defense in depth against a workspace-config change between start and
resume; a two-line fix.

---

## B. Is the substrate sound for a long-running autonomous boss?

The engine, journal, capability intersection and sandbox are sound and I would build on
them. The layer the boss actually runs on is not ready. Highest-risk failure modes over a
multi-hour/multi-day run, ranked:

**B1 — The boss cannot act at all (A1 + A2).** It will produce a plausible plan, request
`bash`/`grep`, and the tool call will either never reach the loop (A1) or return
`capability denied` (A2). Because the run still terminates `Success` on the `ok` port, the
journal will show a *clean, successful* run that changed nothing. This is the worst
possible failure shape for an autonomous system: it looks healthy while doing nothing.
Nothing else on this list matters until it is fixed.

**B2 — No brake on spend or effort.** `BudgetKind::MicroUsd` is *read* once
(`crates/pii-graph/src/run.rs:265`) and **never charged** anywhere in the workspace.
`BudgetKind::ToolCalls` is defined (`crates/pii-core/src/budget.rs:13`) and never charged
either. `max_model_calls` still does not exist in any crate (only in
`specs/06-phase3-mvp-hardening.md:110`). The only live dimension is `Tokens`
(`agent.rs:214-228`), and on the streaming path its counts come from the last NDJSON line
(`ollama.rs:224-232`) — a parse miss silently charges zero. A looping boss on a paid model
has no enforceable ceiling.

**B3 — Unbounded durable growth, no retention.** Three append-only stores with no prune
verb: `RUN_JOURNAL` (by design, documented in `HANDOFF.md`'s rss-gate section), the
per-turn full-transcript session rewrite (A9a), and `HealAttempt.attempts`
(`crates/pii-store/src/heal.rs:37`) which only grows per signature. A daily boss is the
first workload that runs for a month; `pii admin prune` was in the P4.2 plan
(`007-...-plan.md:80`) and was not built.

**B4 — The human-approval contract is not enforceable inside a loop (see E2).** The boss
loop *is* a loop. A gate node re-entered on iteration 2 of the same run reads its own
cached decision and auto-allows. The user's #1 decision ("human approves every promotion")
does not hold in the shape the boss uses.

**B5 — One panic takes down everything, and nothing restarts it.** `panic = "abort"` in
release plus a single daemon means a panicking node kills every concurrent session,
including the boss's own state. Post-P4.2 the crashed run at least surfaces as `Failed`
(`session_durability.rs:165`), but recovery depends on the container `entrypoint.sh` hook
landed in P4.5 — there is no supervision loop that notices "the boss stopped" and says so.

**B6 — Nothing stops the boss from editing its own reward signal.** No guard anywhere in
`crates/` or `xtask/` refuses a patch touching `xtask/`, `quality-gate.sh`, `graphs/`,
`specs/`, `.github/`, or a `#[test]` assertion. The gates *are* the boss's fitness
function; today it has write access to them whenever its `scope_root` is the workspace.

---

## C. `HealStore` schema for the boss's fixing-analysis ledger

**Not enough — it needs the attempt → patch → gate → decision evolution.**

Current schema (`crates/pii-store/src/heal.rs:13-38`): `HealAttempt { signature_hash,
gate, first_seen_ms, last_seen_ms, attempts: Vec<HealAttemptRun { run_id, outcome:
String, ts_ms, detail }> }`. It records **that a gate failed and was seen again**. That is
the right shape for Slice 0 (detect-only) and it is genuinely well-built: blake3 signature
key so repeat failures update one row, no held transaction, bounded cache.

It cannot answer any question the boss's ledger must answer: *what did we change? did it
work? who said yes? what did it cost?*

Recommended evolution (additive; the key stays `signature_hash`):

- `HealAttemptRun` gains:
  - `patch_hash: Option<String>` — the diff stored **content-addressed** in the existing
    blob table (`Store::put_graph`/`get_graph` is already a CAS, `pii-store/src/lib.rs`),
    not inline. Inline diffs would make the row unbounded.
  - `worktree: Option<String>` + `branch: Option<String>` + `base_commit: Option<String>`
    — the scope the fix was made in, so the artifact is reproducible.
  - `gate_before: String`, `gate_after: Option<String>` — the outcome of the *same* gate
    re-run after the patch. This is the single most important missing field: without it
    the ledger cannot distinguish "we tried" from "we fixed it".
  - `decision: Option<Decision { allow: bool, approval_id: String, by: String, ts_ms: u64 }>`
    — bound to the gate node's `approval_id` (`crates/pii-server/src/nodes/gate.rs:101`)
    so the ledger and the approval store agree on one identity.
  - `model: String`, `tokens_in: u64`, `tokens_out: u64`, `micro_usd: u64` — the cost of
    the attempt. This is also what makes B2's budget dimensions chargeable with real data.
- `HealAttempt` gains `state: HealState` — a real enum
  (`Detected | Patched | Validated | Rejected | Approved | Promoted | Reverted`), replacing
  today's stringly-typed `outcome: String` (`heal.rs:18`).
- `attempts` becomes a **bounded ring** (last N, default 20) with a separate
  `attempt_count: u64`. Otherwise a signature the boss retries daily grows one row
  forever (B3).

Do this **before** the promotion slice, not after: the promotion gate's audit trail is the
only reason a human can meaningfully approve anything.

---

## D. The `pii-mcp` client hang — actual cause and minimal fix

**The `HANDOFF.md` caveat is misdiagnosed. The client is not broken; the server never
exits.**

I ran the client against a `tee`-wrapped server and captured both directions of the wire.
The full protocol round-trip completes and the client exits **0**:

```
client -> server:
  {"jsonrpc":"2.0","id":0,"method":"initialize",...}
  {"jsonrpc":"2.0","method":"notifications/initialized"}
  {"jsonrpc":"2.0","id":1,"method":"tools/list",...}
  {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{...,"name":"bash"}}
server -> client:
  initialize result, 7 tools, and a tools/call result
client stdout:
  tools: ["bash","edit","fs_read","fs_write","glob","grep","multi_edit"]
  call bash: Complete(CallToolResult { ... "error: invalid: command must be a string" ... })
  CLIENT_RC=0
```

(The `bash` error is just the smoke test calling the first tool with no arguments —
`run_client` sends `CallToolRequestParams::new(name)` with no args,
`crates/pii-mcp/src/main.rs:189`.)

**Root cause**: the server's `main` parks forever —
`std::future::pending::<()>().await` at `crates/pii-mcp/src/main.rs:155`. The
`RunningService` correctly keeps the server alive, but *nothing ever resolves*, so
`pii-mcp` does not exit on stdin EOF or on transport close. Meanwhile `run_client` builds
the child with `tokio::process::Command` and only `TokioChildProcess` pipes stdin/stdout
(`main.rs:169-171`) — **stderr is inherited**. So after the client returns, the spawned
server survives as an orphan holding the *invoking shell's* fd, and the pipeline never
sees EOF. The shell hangs; the client did not.

PROVEN: four orphaned `pii-mcp` processes with `ppid == 1` are alive in this container
right now (started 04:23, 11:30, 11:34 during the P4.7 session, plus 11:40 from my own
probe). The `tee`-wrapped run left no orphan only because `tee` exits at EOF and SIGPIPEs
the server.

**Minimal fix** (both halves, ~4 lines):

1. Server: replace `std::future::pending::<()>().await` (`main.rs:155`) with
   `let _ = running.waiting().await;` — `RunningService::waiting` exists in this exact
   dependency (`rmcp-3.2.0/src/service.rs:1093`) and resolves with a `QuitReason` when the
   transport closes. Bounded server lifetime, clean exit code, no orphans.
2. Client: set `command.stderr(std::process::Stdio::null())` before
   `TokioChildProcess::new` (`main.rs:169-171`), and `let _ = running.cancel().await;`
   (`rmcp-3.2.0/src/service.rs:1162`) before `run_client` returns, so the child is
   explicitly torn down rather than orphaned.

With (1) in place the self-test becomes a genuine exit-code assertion and can be an
exit criterion, not a manual smoke test. Also worth fixing while there: pass real
arguments to the smoke call (e.g. `fs_read` on a temp file) so the test proves a tool
*ran*, not that an error round-tripped.

---

## E. Prior opus review items — current status

| # | Item | Status |
|---|---|---|
| 1 | resume restores `scope_root` | **FIXED** |
| 2 | approval-gate cycle salt | **PARTIAL — the boss's shape is still broken** |
| 3 | `heal-watch.toml` phantom gate | **FIXED** |
| 4 | `run_ci` missing reachability/doc-drift | **STILL OPEN** |
| 5 | anti-reward-hacking refusal | **NOT PLANNED — no code, no spec** |
| 6 | dev/prod runtime-dir vs worktree | **Recorded as a constraint, no code** |

**E1 — resume scope-root: FIXED.** `spawn_resume` recovers the scope from the journal
(`crates/pii-server/src/runs.rs:138`) via `stored_run_scope_root`
(`runs.rs:177-195`), which reads `RunEvent::Started { input_hash }` and resolves it
through the CAS. The real path does store the input content-addressed
(`crates/pii-graph/src/run.rs:87` → `Started { input_hash }` at `run.rs:99`), so the test
fixture matches production. Two tests (`runs.rs:352`, `runs.rs:395`). Residual gap: no
re-validation on resume (A10).

**E2 — approval-gate cycle: PARTIAL, and the remaining hole is exactly the boss's shape.**
`approval_id` now folds a `cycle` salt (`crates/pii-server/src/nodes/gate.rs:101-116`),
cycle 0 preserves the historical `blake3(run:node)` id so §8.8 replay is unchanged
(test `gate.rs:153-170`), and a different cycle yields a different id
(test `gate.rs:141-148`). That is all correct.

But `cycle` is a **static graph-config field read at build time**
(`gate.rs:56-59`) and nothing increments it at runtime. Inside a single run — which is
what a boss *loop* graph is — a gate node re-entered on iteration 2 has identical
`(run_id, node_id, cycle)`, finds its own cached decision (`gate.rs:79-87`), and
**auto-allows without a human**. The original finding stands for the exact topology P4.7
needs. Getting a fresh cycle today requires spawning a new run from a *different* graph
spec each cycle (the spec is content-addressed, so a changed `cycle` is a new graph id) —
workable, but it is not what "the boss loop" means and nothing implements it.

**Fix**: derive the salt at runtime, not from config — e.g. fold the loop-iteration
counter or an explicit `cycle` value read from `input.payload` into `approval_id`, so a
re-entered gate is always a new decision. Keep the config field as the default of 0.

**E3 — heal-watch: FIXED.** `graphs/starter/heal-watch.toml:27` is now
`gate = "reachability-check"`, a real xtask subcommand. The old `quality-gate` value
would have recorded a phantom permanent failure every 30 minutes.

**E4 — `run_ci` reachability/doc-drift: STILL OPEN.** `xtask/src/main.rs:63-81` runs
deps-check, net-check, lint-files, proto-gen, bench, fmt, clippy and test. Neither
`reachability-check` nor `doc-drift-check` is in the CI gate set — so the two gates the
Phase-4 plan called "the strongest self-improvement machinery in this repo"
(`007-...-plan.md:122`) are still opt-in and not enforced by the gate the boss will be
told to satisfy. This is a one-line fix and it is the cheapest item on this list.

**E5 — anti-reward-hacking: NOT PLANNED.** No refusal logic exists anywhere in `crates/`
or `xtask/`, and no spec mentions it. Given B6 and E4 together, a boss whose patch is
scored by a gate it can edit is a genuine hazard, not a theoretical one. This should be a
mechanical check on `git diff --name-only` inside the heal/promotion graph — escalate,
never propose — and it is cheap because the heal loop already has a worktree and a gate
node.

**E6 — dev/prod runtime-dir vs worktree: constraint recorded, correct, no code.**
`HANDOFF.md` caveat 7 matches the code: `validate_scope_root`
(`crates/pii-server/src/runs.rs:237-259`) canonicalises both sides and rejects anything
not under the workspace, so a `/root/.pii-dev/` worktree can never be a heal
`scope_root`. Keeping `/root/.pii-dev/` as a runtime/store dir only, with worktrees under
the workspace, is the right call and remains unimplemented.

**Also still open from the Phase-4 plan itself**: `MicroUsd` never charged (B2),
`max_model_calls` absent (B2), retention/prune absent (B3), `pii-evolve` crate absent
(`crates/` has no `pii-evolve`).

---

## F. Highest-leverage next P4.7 step, and its single biggest risk

**Step: not `pii-evolve`. A "P4.7.0 — the agent loop actually works, proven live" slice.**

Contents, in order:
1. A1 — accumulate `message.tool_calls` in `OllamaProvider::chat_streaming`
   (`ollama.rs:228`).
2. A2 — pass the caller's `Ctx` (cloned) into the bounded tool thread
   (`agent.rs:412-417`).
3. A3 — dechunk inside `NetService::request_streaming` (`service.rs:395-418`) and buffer
   partial NDJSON lines across chunks in the Ollama delta closure
   (`ollama.rs:192-203`).
4. E4 — add `reachability-check` + `doc-drift-check` to `run_ci` (`xtask/src/main.rs:63`).
5. D — bounded MCP server lifetime (`main.rs:155`) so the self-test is an exit code.

And **one non-negotiable exit criterion**: a live run against the local Ollama in which
the agent calls `grep` and then `bash`, and `pii graph trace <run_id>` shows `ToolCall` +
`ToolResult` events and at least two `AssistantDelta` events. Not a fake transport. Not a
scripted provider.

Rationale: every P4.7 capability — localize, patch, validate, escalate, promote, research,
open a PR — is a tool loop over `bash`/`grep`/`edit`. Building the `pii-evolve` crate on
top of a loop that returns at turn 1 with `Terminal::Success` would produce a boss whose
journal is full of clean successful runs that changed nothing, and the failure would be
attributed to the model, the prompt, or the graph for a long time before anyone suspected
the substrate.

**Single most important risk of that step: it is only provable with a live model, and the
existing test apparatus is structurally incapable of catching these bugs.** All three
defects are invisible to the 52 test binaries because every fake bypasses the broken path:
the default `chat_streaming` delegates to `chat` (`pii-model/src/lib.rs:132-141`),
`ChunkedTransport` hands unframed NDJSON (`ollama.rs:333-377`), and `SlowTool` ignores its
`ctx` (`agent.rs:572-594`). If this fix is validated the way it was written — with fakes —
it will **vacuous-pass**, exactly as spec 16 part 2 and spec 20 already did per
`HANDOFF.md` caveat 8. The gate must be a live run, and the new acceptance tests must
model the wire: chunk-framed bytes split mid-line, and a tool that actually enforces
`ctx.caps()`.

---

## One-line verdict

**NEEDS WORK** — the single most important thing to fix next is the agent tool loop
(A1 `chat_streaming` drops `tool_calls`, A2 `call_tool_bounded` substitutes an empty
`CapSet`), gated on a **live-model** exit criterion; do that before any `pii-evolve` code,
because today every autonomous run would report success while doing nothing.


---

## [merged] Pii (`pi-improved`) — architecture-conformance review + next-phase proposal

<!-- Absorbed from `026-codebase-analysis-and-phase5-plan.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

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
