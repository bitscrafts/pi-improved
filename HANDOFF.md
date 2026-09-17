# Pii — Session Handoff

## CURRENT STATE — 2026-09-17: Phase 5.0 COMPLETE + Phase 5.1 Track A COMPLETE (A2, A3, A4, A5 all landed)

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
landed earlier) through A5. Phase 5.2 (structural debt ratchet, incl. the
`agent.rs` split) is next — see 026 report's own sequencing rationale.

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
