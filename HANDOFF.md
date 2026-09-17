# Pii — Session Handoff

## CURRENT STATE — 2026-09-17: Phase 5.0 COMPLETE + Phase 5.1 Track A COMPLETE + Phase 5.2 structural-debt/warn-band ratchet COMPLETE (F-13, F-14, all six F-12 warn-band files)

Full history before the 2026-09-16 pin lives at
`_docs/99-archive/handoff-2026-09.md` (2002 lines, archived intact —
nothing was deleted, only moved out of the live file per Phase 5.0.7
hygiene).

**Gate status: `./quality-gate.sh` exits 0**, verified with A2+A3+A4+A5
merged into the Phase 5.0 baseline (`_tmp/gate-spec42-final.log`).

**Found and fixed while verifying A4**: `/root/.local/bin/pii` and
`/projects/.local/bin/pii` were stale (pre-dated spec-40's `fail` port),
causing `pii graph lint` to report false `V1: node has no port 'fail'`
errors against a genuinely correct graph. Rebuilt from source
(`cargo build -p pii-cli --bin pii`) and reinstalled both paths — this is
the same install convention pidag's own binary uses (hash-verify both
paths after any `pii-cli` change, not just after a `pidag` change).

**RETRACTED (2026-09-17): "`pi --provider` routes to Anthropic" was a false
alarm from a malformed manual repro** (prompt placed immediately after
`-p`, which pi's CLI misparses — see `/projects/pidag/docs/FINDINGS.md`).
pidag's real invocation shape (`pi_print.rs`, prompt always last) works
correctly. A5 was implemented end-to-end through the REAL `pidag sdd --run`
worker chain (pi → ollama-cloud) once this was corrected and pidag was
launched via a self-contained script pinning `OLLAMA_API_KEY`/`PATH`
explicitly (see the `pidag` skill's new "Standardized run procedure"). Specs
39-41 remain implemented via subagent dispatch as originally recorded below
— not re-run, no reason to.

### Phase 5.1 Track A (failover) — COMPLETE

pidag harness had two real defects (fixed 2026-09-17, see FINDINGS.md):
`validate-baseline` hard-failed the whole DAG despite having zero
dependents (fixed via `Node.allow_failure`), and a stale `/usr/local/bin/
pidag` binary shadowed the correct `/root/.local/bin/pidag` on PATH in some
invocations (removed). Specs 39-41 were implemented via direct subagent
dispatch (predating both fixes); spec 42 (A5) was implemented via the real
`pidag sdd --run` worker chain once both were resolved — see its own row
below for the full run trace.

| Spec | Item | Status | Evidence |
|---|---|---|---|
| 39 | A2 — structured provider failure (PiiError::Provider(ProviderFailure), classify_provider) | **DONE** | `pii-abi`: `ProviderFailure`, `PiiError::Provider` appended last, `classify_provider`. `pii-model`: Ollama/llama.cpp construct it with real HTTP body / context-overflow text instead of a formatted string. 10 named tests (T1-T10) all independently re-run and pass. Full gate green post-merge (`_tmp/gate-spec39-merged.log`). One stray untracked throwaway test (`zz_verify_d3_roundtrip.rs`, self-marked "not for commit", D3/D6 dev scratch unrelated to spec-39) was found polluting `cargo test --workspace` and moved to `_tmp/_trash/` (never deleted). |
| 40 | A3 — failure-as-payload (AgentNode fail port, FailurePayload) | **DONE** | `pii-abi`: `FailurePayload{status,reason:FailoverReason,provider}`. `pii-server/agent.rs`: `fail` port appended last after ok/err/budget; `route_provider_failure()` classifies `PiiError::Provider` to `fail`, everything else keeps the pre-existing `err` shape unchanged. 6 named tests (T1-T6, incl. the literal 026 acceptance test) independently re-run and pass. Full gate green post-merge (`_tmp/gate-spec40-merged.log`). Two honest deviations reported and verified: T4's classified reason is `Overloaded` not `ServerError` (spec prose was wrong, test is right); `xtask lint-files` pre-existing hard-error on `xtask/src/silent_noop.rs` unaffected. |
| 41 | A4 — failover ladder graph (graphs/starter/failover.toml) | **DONE** | New `graphs/starter/failover.toml`: `primary` (agent, `loop_budget=3`) -> `route_failure` on `$.payload.reason` -> `RetrySame` reasons loop back via `retry_primary`, `Compress` reasons dead-end at `compress` (deliberate — 026's own "never advances the chain" acceptance line), everything else falls through to `secondary` -> one more level, ending at `exhausted`. New `crates/pii-server/tests/failover_graph.rs` (278 lines), 3 named tests (T1-T3) independently re-run and pass; `pii graph lint` independently re-run (exit 0) after fixing the stale-binary issue above. Two real engineering findings self-reported and verified: `loop_budget=3` not 2 (the scheduler's re-entry accounting makes a budgeted node's *Nth* execution unconditionally terminal, so `loop_budget=N` allows only `N-1` executions whose result matters — an off-by-one a naive read of `heal-fix.toml`'s `loop_budget=2` would miss); `secondary` deliberately has no retry back-edge (a non-entry node with both a fresh and a loop-back inbound edge can deadlock when the loop-back edge is permanently Dead — a real scheduler constraint worked around structurally, not by touching the engine, per G1/G3). Full gate green post-merge (`_tmp/gate-spec41-merged.log`). |
| 42 | A5 — failover reason operator surface (`pii graph trace --reason`, doc 006 update) | **DONE** | `crates/pii-cli/src/cmd_trace.rs`: new `--reason <node-id>` flag prints the byte-identical `FailurePayload` from the latest `fail`-port event for that node (`no-failure` when the node never failed, or when the latest event across retries recovered) — 335 lines (≤450), zero `unwrap/expect/panic!/todo!/unimplemented!`. `_docs/06-research/006-pii-graph-engine.md` gained a new "## 10. Failover reason on the fail port" section (old §10 renumbered to §11). 4 named tests (`trace_reason_prints_no_failure_for_never_failed_node`, `trace_reason_prints_failure_payload_for_fail_port`, `trace_reason_uses_latest_event_across_retries`, `trace_reason_missing_node_prints_no_failure`) independently re-run and pass. **Implemented end-to-end through the real `pidag sdd --run` worker chain** (iter1 free model got 11/14 exit criteria; iter2 paid model — `--allow-paid` — fixed the remaining 3: the two doc-grep criteria and one stray `unwrap()` in a test), the FIRST Track-A spec to go through pidag rather than a subagent, closing the loop on this session's own harness fixes. Full gate green post-merge (`_tmp/gate-spec42-final.log`, `EXIT=0`). |

Track A (failover becomes real) is now fully done: A1 (spec 37, classify,
landed earlier) through A5. Phase 5.2 (structural debt ratchet) is next.

### Phase 5.2 (structural debt ratchet) — COMPLETE

| Spec | Item | Status | Evidence |
|---|---|---|---|
| 43 | Split `agent.rs` into `agent/{mod,turn,tools,memorize,tests}.rs` | **DONE** | Pure move, no behavior change: `AgentNode`/`AgentFactory`/`NodeFactory` impl/`resolve_model`/`route_provider_failure` stay in `mod.rs`; the turn loop moves to `turn.rs`; `run_tool_calls`/`call_tool_bounded`/`emit_live` move to `tools.rs`; `split_memorize`/`history_messages` move to `memorize.rs`; the test module moves verbatim to `tests.rs` (self-gated via an inner `#![cfg(test)]`, prompting a small `xtask/src/lint_files.rs` extension to recognize that shape as zero effective lines). `registry.rs` (the only external call site) untouched. New `agent_line_gate.rs` regression test. Implemented via the real `pidag sdd --run` worker chain, iter1/free model, 12/12 exit criteria first try — no paid escalation needed. Full gate green (`_tmp/gate-spec43-final.log`, `EXIT=0`). Committed `a337ab2`. |
| 44 | Split `xtask/src/silent_noop.rs` into `silent_noop/{mod,scans,helpers}.rs` | **DONE** | Pure move, no scan-logic change: `AllowEntry`/`allowlist()`/`run()`/`report_lines()`/tests stay in `mod.rs`; the five scans move to `scans.rs`; shared ID/parsing utilities move to `helpers.rs`. `main.rs`'s `mod silent_noop;` and its `silent_noop::run(` call spelling (self-asserted by `run_ci_includes_self_improvement`) untouched. `silent-noop-check --list` output independently diffed byte-identical before/after. **This was the only hard `xtask lint-files` violation this session (458 effective lines, 8 over the 450 cap) — `lint-files` now exits 0 for the first time.** Implemented via the real `pidag sdd --run` worker chain: iter1 (free model) hit 11/12 criteria, the one miss was the spec's OWN exit-criterion bug (a blanket `unwrap/expect` grep across all 3 new files that didn't exclude the test module, same as the original file's own pre-existing, allowed `.expect("workspace root")` inside `#[cfg(test)]`) — fixed the spec, re-verified manually rather than re-running iter2/paid unnecessarily since the implementation itself was already correct. Full gate green (`_tmp/gate-spec44-final.log`, `EXIT=0`). Committed `8988f5f`. |
| 45 | Split `pii-net/src/service.rs` into `service/{mod,conn,http,dechunk}.rs` | **DONE** | Pure move, no behavior change: `NetConfig`/`NetService` (`new`/`connect`, the authorize→pin→grant→dial chokepoint) stay in `mod.rs`; `Stream`/`Conn` (incl. `Drop`, independently verified byte-identical — this is where D1's TLS-handshake lease-leak fix lives) and `GrantInfo` move to `conn.rs`; the `HttpTransport` impl moves to `http.rs`; the chunked-decoder free functions move to `dechunk.rs`. `lib.rs`'s public re-export list verified unchanged and checked against every external call site. Implemented via the real `pidag sdd --run` worker chain (iter1 free model, 12/13 criteria, ~16-18 min real runtime — the longest of the three module splits, expected given raw-socket/TLS complexity); the one miss was a genuine clippy lint CONFLICT the move itself introduced (moving `Inner`/`Registry` into the now-private `conn.rs` submodule made `redundant_pub_crate` want `pub` while `unreachable_pub` then rejected that `pub` — a real interaction the original flat file never hit) — fixed directly rather than escalating to the paid model (`--allow-paid` was exhausted for this run and the fix was mechanical: `pub(crate)` + a scoped `#[allow(clippy::redundant_pub_crate)]`). Full gate green (`_tmp/gate-spec45-final.log`, `EXIT=0`). Committed `629ac9f`. |
| 46 | F-14: move dead `HealAttempt` product fields into generic `extra` | **DONE** | `worktree`/`branch`/`base_commit`/`micro_usd` were declared but never written anywhere outside two struct-literal fallbacks — confirmed dead by grepping every field access workspace-wide. Removed and replaced with `extra: serde_json::Map<String, serde_json::Value>`; `patch_hash`/`model`/`tokens_in`/`tokens_out` kept (real writers via `record_repair`). Codec confirmed plain `serde_json` (not positional/bincode) so field removal is backward-compatible with rows already on disk; regression test proves an old-shape payload with the removed keys present still decodes cleanly. A third literal site (`crates/pii-server/tests/spawn_node.rs`) also needed updating, found only via independent `git diff` review, not the implementer's own summary. Implemented via a Claude Code subagent per explicit user request ("no pidag this time" — the alternate-implementation comparison test) rather than pidag, since this was a real schema/persisted-redb-data redesign, not a pure move. Full gate green (`_tmp/gate-spec46-final.log`, `EXIT=0`). Committed `839ae38`. |
| 47 | F-13: reorganize `pii-server` into `engine/`, `node_kinds/`, `daemon/` module boundaries | **DONE** | Scoped as a module reorganization INSIDE `pii-server` (not a 3-crate split — `dispatch.rs`'s engine-bridge concern already calls `daemon::methods` for ~10 JSON-RPC methods, a real coupling a full crate split would need to resolve first; that tradeoff was recorded for a future opus feasibility review rather than attempted here). `engine/` now holds `bus`, `runs`, `dispatch`, `subscribe`, `registry`, `session_durability`, `conn`, `uds`, `loopback`, `tool` (moved via `git mv`, several byte-identical); `node_kinds/` holds `agent/`, `nodes/`, `compaction.rs` (confirmed used only by `agent/turn.rs` before moving). `daemon/` untouched. Every `crate::X`/`pii_server::X` reference across `pii-server`, `pii-cli`, `pii-mcp` updated to the new paths; no public API or test assertion changed. The `dispatch.rs → daemon::methods` coupling is now documented explicitly in `engine/mod.rs` rather than left implicit. Implemented via a Claude Code subagent (45 api_calls, ~12 min) per user direction to test the alternate path for F-12/F-13; independently verified (git status, gate rerun) before commit. Full gate green (`_tmp/gate-spec47-final.log`, `EXIT=0`). Committed `39db2e1`. |
| 48 | F-12: split `pii-store/src/mem.rs` into `mem/{mod,write,read}.rs` | **DONE** | Pure move: `MemStore`/`PutRows`/`Posting`/`err()` helper plus the lifecycle/counter methods (`open`, `n_docs`, `sum_dl`, `dims`, `meta`, `bump_meta`) stay in `mod.rs`; write-path methods (`put`, `doc_exists`, `bump_df`, `archive`) move to `write.rs`; read-only query methods (`postings`, `df`, `doc`, `doc_ids`, `vector`, `vector_batch`, `edges`, `concepts`, `archived`) move to `read.rs`, each as a separate `impl MemStore` block. `pii_store::mem::MemStore` resolves identically. First warn-band file done entirely via pidag after the switch back from subagents (user: "73% on anthropic! ... use mainly pidag!"). 11/11 exit criteria first try, iter1 free model. Full gate green (`_tmp/gate-spec48-final.log`, `EXIT=0`). Committed `aaee53c`. |
| 49 | F-12: split `pii-graph/src/run.rs` into `run/{mod,engine,resume}.rs` | **DONE** | Pure move: `mod.rs` keeps `run`/`run_with_validate`; `engine.rs` holds the private `Engine` struct and its `tick_loop` (widened to `pub(super)`, needed by both `mod.rs` and `resume.rs` to construct the struct and call the loop) plus small helpers left private; `resume.rs` holds `resume` unchanged. `tick_loop`'s body moved whole, not split further, honoring its own pre-existing author comment against splitting it. `pii_graph::run::{run,run_with_validate,resume}` resolve identically for both external callers (`pii-cli`'s `cmd_run.rs`, `pii-server`'s `engine/runs.rs`). Via pidag, 12/12 exit criteria, iter1 free model. Full gate green (`_tmp/gate-spec49-final.log`, `EXIT=0`). Committed `11df84d`. |
| 50 | F-12: split `pii-tui/src/lib.rs` into `{state,render,input,driver}.rs` | **DONE** | Pure move into four sibling files: `state.rs` (`TuiError`/`ConnStatus`/`LogLine`/`AppState`+impls), `render.rs` (`render`/`render_log_line`/`render_event_line`), `input.rs` (`KeyOutcome`/`handle_slash_command`/`handle_key`/`init_session`), `driver.rs` (`run_tui`/`_reconnect_hint`+tests). Via pidag: iter1 hit 11/12 (clippy `redundant_pub_crate` on `input.rs`'s `pub(crate)` items inside a private module, plus a `too_many_lines` on `render_event_line` that grew slightly past 80 during the move) — fixed manually (matches the spec-45 clippy-conflict pattern: `#[allow(clippy::redundant_pub_crate)]` + a scoped `#[allow(clippy::too_many_lines)]` with rationale comments) rather than escalating to paid. `pii_tui::run_tui` (the crate's one real external call site, `pii-cli`'s `cmd_tui.rs`) resolves identically. Full gate green (`_tmp/gate-spec50-final.log`, `EXIT=0`). Committed `b98a581`. |
| 51 | F-12: extract `xtask/src/main.rs`'s argv parsers into `cli_parse.rs` | **DONE** | Pure move of exactly four self-contained functions (`parse_mem_eval`, `parse_worktree_flag`, `parse_promote_collect`, `parse_rss_gate`) into `cli_parse.rs` as `pub` (xtask's `unreachable_pub`/`redundant_pub_crate` lints require full `pub` for a binary crate's sibling module, unlike the `pub(crate)` pattern used in library crates). `main()`, `run_ci`, `report`, `run_cmd`, `workspace_root`, and both test constructs (the `mod top_level_tests` block and the standalone top-level `deps_check_rejects_redb_outside_store` test, deliberately kept separate for `-- --exact` path matching) left untouched. Verified via `cargo test -p xtask` (43 passed) plus a manual CLI smoke check. Via pidag, 15/15 exit criteria, iter1 free model. **`xtask lint-files` now reports zero warnings across the entire workspace** — this closes Phase 5.2's F-12 warn-band ratchet completely (last of six files; `agent.rs`/`silent_noop.rs`/`service.rs` done via specs 43-45, `mem.rs`/`run.rs`/`pii-tui/lib.rs`/`xtask/main.rs` done via specs 48-51). Full gate green (`_tmp/gate-spec51-final.log`, `EXIT=0`). Committed `a8b9af4`. |

Phase 5.2 as scoped by the 026 report is now COMPLETE: F-13 (module
reorganization, chosen over a full 3-crate split per the documented
`dispatch.rs → daemon::methods` coupling tradeoff — the alternatives were
recorded in agent-memory for a future opus feasibility pass, not
discarded), F-14 (dead-field cleanup), all six F-12 warn-band files, and
D4/D7 (sync-blocking seam) are done and independently gate-verified.
Remaining structural items are genuinely NEXT-PHASE scope: F-11
(357-site string-typed `PiiError`, a Phase 5.1 planner decision already
tracked separately), F-18/F-19/F-20 (test coverage gaps: `pii-proto`
round-trips, `pii-store::MemStore` contract, `pii-tools`, `pii-web`
handlers), F16's bounded accept pool (`pii-net/src/listen.rs:190`), F-22
(`real_granite.rs` self-skip unverified).

| Spec | Item | Status | Evidence |
|---|---|---|---|
| 52 | D4/D7: channel poll off the tokio executor, worker kill off the state lock | **DONE** | D4: `router.rs`'s `spawn_all` wrapped the fully-synchronous `Channel::poll` in `tokio::task::spawn_blocking` — a slow network/file-backed poll no longer starves the async executor. D7: `Supervisor::death`/`::reload` restructured to release the `state` mutex before the blocking `ChildProc::kill()` (which blocks on `wait()`). **Two real regressions caught during independent re-verification, not visible in the implementer's own reported diff**: (1) a same-thread mutex re-lock deadlock from taking `slots.lock()` twice inside one tuple expression (Rust's temporary-lifetime-extension kept the first guard alive), caught by re-running the PRE-EXISTING `kill9_fails_in_flight_with_workerdied` test and watching it hang for the first time; (2) the crash-loop-cap's early return was dropped, so a `Failed` worker still respawned — caught by `restart_storm_is_capped` asserting `Failed` and getting `Ready`. Both fixed, plus two clippy findings (a second lock-held-across-kill site, a pedantic `map().unwrap_or()`). Implemented via pidag/pi (glm-5.3-flash free tier, iter1) — the run itself hung 20+ minutes on a SEPARATE pidag bug (unbounded output-drain wait, fixed separately in pidag commit `2db967c`) before this content was recovered from the worker's session log and independently verified/fixed/committed. Full gate green (`_tmp/gate-spec52-final2.log`, `EXIT=0`). Committed `b48b497`. |

### Phase 5.0 re-baseline (2026-09-16) — COMPLETE

This session executed the 026 report's Phase 5.0 punch list
(`_docs/06-research/026-codebase-analysis-and-phase5-plan.md`, Part 3)
autonomously per user direction ("fix all of them to get into phase 5, I
will verify it later"). **Nothing is committed** — all work below is in the
working tree for review (AGENTS.md hard rule: the workhorse/this pass never
commits).

**Gate status: `./quality-gate.sh` exits 0.** Verified warm (multiple runs)
and cold (`cargo clean` immediately before). This is Phase 5.0's hard
prerequisite (5.0.1) and it is now genuinely met — see F-01 row.

### Ledger (F-xx / D-x from the 026 report)

| ID | Item | Status | Evidence |
|---|---|---|---|
| F-01 | Quality gate RED (E0463 doctest) | **DONE — verified GREEN** | Root cause: `cargo clippy --all-targets` and `cargo test`'s doctest step shared `target/debug/deps` and raced on schemars/serde_json rlib hashes; a second, independent bug also surfaced on a clean build — `pii-worker`'s supervisor tests spawn `target/debug/pii` directly and need it pre-built. Fix in `quality-gate.sh`: isolated clippy (`CARGO_TARGET_DIR=target/clippy-gate`) and doctests (`CARGO_TARGET_DIR=target/doctest-gate`) into their own target dirs, split `cargo test` into `--lib --bins --tests` then `--doc` as separate steps, added `cargo build --workspace` before testing. **Verified multiple times, including one full post-`cargo clean` run with D1+D2+D3/D6 all merged** — see `_tmp/gate-final-postclean.log`. One earlier warm run in an isolated 3x loop hit an unrelated pre-existing timing flake (`session.prompt took 119ms, must be <50`) under concurrent build load — not a correctness regression. |
| F-02 | AGENTS.md workflow drift | DONE | Corrected: `max_iterations` (3→"2, escalate then stop"), `crates/` "(when Phase 0 lands)" → landed/18 crates, single-binary claim → 4 binaries, `pii-hw` unsafe carve-out → never built, workspace-wide forbid confirmed via `Cargo.toml:54`. |
| F-03 | specs/33 edited by non-planner | DONE | Adopted as a planner-authored change: added an explicit `> **Amendment (planner, 2026-09-16...)**` block to `specs/33-slice-e-live.md`. |
| F-04 | spec-33 R5 unverifiable exit criterion | DONE | Same edit as F-03 — R5 now documents the script's absence and points at the real in-crate proof. |
| F-05 | specs 35/36 repurposed, no tracking spec | DONE | Spec 38 (HTTP fetch via pii-net) landed earlier this thread. Spec 39 (research-writes-memory) confirmed absent — matches the 026 plan's own recommendation to cut it. |
| F-06 | HANDOFF.md bloat (139KB/54 sections) | DONE | Archived full history to `_docs/99-archive/handoff-2026-09.md`; this file is now pinned CURRENT STATE + ledger only. |
| F-07 | feature-gating.md fictional flags | DONE | Added a "Current reality" section: none of the documented features exist yet; only 3 test-only features are real. |
| F-08 | pii-mcp-bridge.md wrong gating claim | DONE | Corrected: `pii-mcp` is unconditional (`Cargo.toml:20`), not feature-gated. |
| F-09 | "single static binary" claim false | DONE | AGENTS.md now lists all 4 binaries. |
| F-10 | Stale refs (pii-hw, pii_obs, INDEX.md, node.rs "four") | DONE | `clippy.toml`, `_docs/INDEX.md`, `pii-graph/src/node.rs:64`, AGENTS.md all corrected. |
| F-11 | 357-site string-typed PiiError | NOT STARTED | Phase 5.1 planner decision (A2) per the 026 plan, not Phase 5.0 scope. |
| F-12–F-14 | File-size/pii-server-fusion/heal.rs debt | NOT STARTED | Phase 5.2 scope, after Track A (5.1) lands. |
| F-15 | ~40 hardcoded `/projects/pi-improved` test paths | DONE | Verified via a cfg(test)-aware scan (not naive grep): every remaining hit is inside a `#[test]`/`tests/` body. The one non-test hit (a `registry.rs` doc comment) was rewritten. **Zero hits in live code.** |
| F-16–F-20 | Sync-blocking seam / test coverage gaps | NOT STARTED | Phase 5.2/5.3 scope. |
| F-21 | 86GB untracked disk | PARTIAL | `_tmp/_trash/pidag-2026-09-16/` created; moved 19 files (~2.9GB): 15 `pidag.redb.stale-*` backups (~24MB) + 4 `.events.jsonl` over 100MB (1.1GB/836MB/666MB/334MB). `.pidag` went 13GB→11GB. **Target (<3GB) NOT met**: the *active* `pidag.redb` itself is 9.8GB — that's live state (needs pidag-level compaction/vacuum, not filesystem hygiene) and was correctly left untouched, along with 3 non-`stale-`-named phase2 backups (~2.1GB) that didn't match the move pattern. Flagging back: hitting <3GB needs a pidag compaction pass, out of scope for this session. |
| F-22 | real_granite.rs self-skip unverified | NOT STARTED | Out of Phase 5.0 scope. |
| D1 | TLS lease single-exit-path leak (`pii-net`) | **DONE** | `service.rs` dial+TLS path folded into one `Result` before the match so any failure decrements the lease; new test `tls_handshake_failure_releases_lease` in `pii-net/tests/egress.rs` (RED-confirmed on the original bug, GREEN with the fix). Independently re-verified: `cargo test -p pii-net --features testing` all pass, clippy clean. |
| D2 | Channel fs-scope hardcoding | **DONE** | `daemon/mod.rs` gained `resolve_channels_scope()` (resolves under `runtime_dir`, creates it, fails loud on error instead of silent-deny — moved out of `DaemonCtx::build` into its own function to also fix a clippy `too_many_lines` regression); `daemon/router.rs` test `channel_ctx_scope_denies_outside_allows_inside` uses a portable `CARGO_MANIFEST_DIR`-relative path. Independently re-verified: `cargo test -p pii-server --lib daemon::router` passes, clippy clean. |
| D3/D6 | Session durability (append-per-message + cap) | **DONE** | `pii-store`: new `SESSION_MSG` (append-only `(session_id,seq)→bytes`) and `SESSION_SNAPSHOT` tables, `append_session_message`/`session_messages[_since]`/`put_session_snapshot`/`latest_session_snapshot`. `pii-core`: new `ObsEvent::PersistError`. `pii-server`: `Engine` gained an attachable `Observer`; `session_durability.rs` rewritten — `persist_session` now writes meta-only (no transcript), `append_message` appends one row + snapshots every 32 messages, every store-write failure routes through `report_persist_error`→`ObsEvent::PersistError`; `trim_session_runs` does LRU eviction against new `EngineConfig.session_runs_cap` (default 256, `[core].session_runs_cap` in daemon config). New tests: `session_message_append_is_not_quadratic` (200 appends, bytes ≤ 2× final size — proves it's not O(n²)), `session_messages_since_returns_only_tail`, `session_snapshot_latest`, `del_session_clears_messages_and_snapshots` (pii-store); `persist_failure_emits_obs_event`, `session_runs_capped_with_lru_eviction` (pii-server). Post-landing I fixed 4 clippy pedantic lints the subagent's diff introduced (cast-precision-loss/cast-possible-truncation in the new test, doc-markdown backticks, needless-pass-by-value + collapsible-if in `append_message`, implicit-clone in a test) — all confirmed clean afterward. |

### Incident this session (resolved, no data lost)

A D2 subagent ran a **bare, unscoped `git stash`** (no pathspec) to isolate
an unrelated pre-existing test failure it was investigating. Since this
working tree is shared, that stash swept up every other agent's (and the
orchestrator's) uncommitted changes tree-wide — including this session's
SSOT/gate fixes and a D1 subagent's in-progress work. Recovered fully by the
orchestrator: verified via `diff` that all files matched their pre-incident
state byte-for-byte before resuming, nothing was lost. All subagents were
steered afterward with an explicit rule: never run bare `git stash`/
`checkout .`/`reset --hard` in a shared tree — only scoped
`git stash push -- <own files>`.

### What this session did NOT do

- D4/D5/D7 (Phase 5.2 items), F-11 through F-14, F-16 through F-20, F-22 —
  explicitly out of Phase 5.0 scope per the 026 plan's own sequencing.
- **No commits.** Per AGENTS.md hard rule 8, nothing above is committed —
  it's all uncommitted working tree, for the user's review and decision.

### Immediate next steps (in order)

1. User review of the full diff (`git status`/`git diff`) and decision on
   what to commit. All Phase 5.0 items (F-01 through F-10, F-15, D1, D2,
   D3/D6) are implemented and gate-verified; F-21 is partial (documented
   why); F-11 onward are correctly deferred to Phase 5.1+.
2. Once committed, the 026 plan's "harness decision gate" applies: since
   D1/D2/D3-D6 landed via subagent dispatch (not `pidag sdd`), `AGENTS.md`'s
   pidag-as-executor framing may need a truthfulness pass — see the 026
   report's "Harness decision gate" note at the end of its Phase 5.0 section.
3. Phase 5.1 (Track A: failover, starting with F-11's `PiiError` structured-
   error planner decision) can begin once the user green-lights it.

## Prior history

See `_docs/99-archive/handoff-2026-09.md` for the full session-by-session
history through 2026-09-15 (spec 01–38 implementation notes, fork-bomb
incident, model-escalation config, Opus/026 analysis derivation, etc.).
