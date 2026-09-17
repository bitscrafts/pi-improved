# Pii — Codebase Analysis, Debt/Risk Assessment & Phase 5 Plan

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
- **Docs**: `_docs/01-overview/001-pii-vision.md`; `_docs/02-guides/{feature-gating,pi-local-llamacpp,pidag-way-of-work,pii-mcp-bridge}.md`; `_docs/06-research/001…026`; `_docs/manuals/{technical,user}-manual.{tex,pdf}`; `_docs/{INDEX,TOC}.md`.
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
