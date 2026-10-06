<!-- Filed 2026-10-06T16:25:17Z from an Opus plan-mode analysis run.
     Produced by: Claude Code + claude-opus-5-5, plan mode, READ-ONLY (no project writes).
     Brief: /projects/pi-improved/_docs/06-research/041-BRIEF-full-review.md
     Inventory the report was measured against: /projects/pi-improved/_docs/06-research/041-APPENDIX-codebase-inventory.txt
     Repo HEAD at analysis time: d713593 (tree: 3 changed paths)
     Run: session ef17e6d6-1741-42d6-87c6-16ffc775d231, stop_reason n/a, turns 21, cost $15.421959200000002

     STATUS: NOT a spec and NOT a plan of record. Nothing in this document is authorised work.
     Every proposal needs its own pre-registered gate before implementation, and any idea previously
     closed by a measurement stays closed until a NEW measurement reopens it.

     Anything marked "asserted-from-memory" is NOT verified — treat it as a lead, not a fact. -->

# pi-improved full review, run 2026-10-06 (completes and supersedes 031/032)

A copy of this report is at `/projects/_tmp/pii-review/opus-review-result-20261006.md`.

Provenance: Opus review, test mode, resumed session ef17e6d6. Brief: `_docs/06-research/041-BRIEF-full-review.md` with the 2026-10-06 amendment. Tree: HEAD `d713593`. Code is unchanged since `55a8328` (spec-69, 2026-09-18). `git log 55a8328..HEAD -- crates xtask Cargo.toml Cargo.lock quality-gate.sh scripts clippy.toml` prints nothing.

Read-only held. No tracked file was edited by this run, nothing was committed, and no dependency was changed. All mutation and demonstration work ran in scratch exports made with `git archive` under `/projects/_tmp/pii-review/`. At the end, `git status` shows only the brief, the 042 addendum and the tombstone file, all written by the orchestrator, not by this run.

**Evidence labels used throughout:**

| label | meaning |
|---|---|
| **C-run** | I ran a command and read its output |
| **C-read** | I read the source lines myself |
| **C-delegated** | a delegated run executed commands; its logs are on disk |
| **I** | inferred from a delegated read-only review, not re-checked by me |

## Verdict in five lines

1. All six gate commands ran this time. The verbatim test step failed once on a load-sensitive timing flake; re-run at normal load, the full suite is clean: 63 binaries, 536 passed, 0 failed, 10 ignored.
2. Both 2026-09-18 gaps are closed. One residue remains: the original run never proved its tests would fail on regression, and four of the eight spec fixes turn out to be only partly guarded.
3. Two new bugs are proven by failing tests. One is a panic and spurious errors in the HTTP chunked decoder that every streamed model turn passes through. The other is a panic in the bash tool's output renderer on multibyte output.
4. The `reachability-check` gate is proven blind. It passes with both of its real callers deleted. A two-string correction fixes it. A graphify-style call graph would make it worse, because it lacks both caller edges.
5. Rulings: the code-graph veto holds, so no native code-graph service and the harness does not consume one. The harness shape is approved with two amendments. F-13 is confirmed. Modularity and Chromecast are deferred. pidag-as-subset is confirmed with one amendment.

## 1. Gate results

Logs are in `/projects/_tmp/pii-review/gate-20261006/`, files `1-fmt.log` to `6-lint.log`, `4b-test-nofailfast.log` and `bw-*.log`. The toolchain is cargo and rustc 1.95.0, matching `rust-toolchain.toml`. No command was denied.

| # | command (verbatim) | exit | interpretation |
|---|---|---|---|
| 1 | `cargo fmt --all --check` | 0 | Empty diff. C-run |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 221 crates checked fresh in 1m20s, zero warnings. The 88-line "`println` does not refer to a reachable macro" noise from September is gone (spec-67). C-run |
| 3 | `cargo build --workspace` | 0 | Zero warnings. C-run |
| 4 | `cargo test --workspace --lib --bins --tests` | **101** | One failure, quoted below. Cargo stops at the first failing binary, so only 36 of 63 result lines were produced. C-run |
| 4b | the same command plus `--no-fail-fast`, load 3.6 | 0 | 63 binaries, 536 passed, 0 failed, 10 ignored. C-run |
| 5 | `cargo test --workspace --doc` | 0 | 17 lib targets, 0 doctests. The step compiles everything and asserts nothing. It ran in the shared target dir this time, and the F-01 E0463 race did not recur. C-run |
| 6 | `cargo run -q -p xtask -- lint-files` | 0 | `lint-files: 184 files checked, max effective 360 lines (file: crates/pii-store/src/lib.rs)`. C-run |

**Full failure output of step 4** (`4-test.log:590-606`):

```
test boss_watch_reaches_detect_after_resume ... FAILED
failures:
---- boss_watch_reaches_detect_after_resume stdout ----
thread 'boss_watch_reaches_detect_after_resume' (586663) panicked at crates/pii-server/tests/boss_watch.rs:211:9:
timed out waiting for journal growth
failures:
    boss_watch_reaches_detect_after_resume
test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.09s
error: test failed, to rerun pass `-p pii-server --test boss_watch`
```

**What the failure means.** It is a timing flake, not a regression. The load average was 22.8 on 32 cores while scratch builds ran in parallel. The test polls with a fixed 10-second wall-clock deadline (`boss_watch.rs:205`) and normally takes about 7 seconds. Re-run alone 10 times as load fell from 15 to 6, it passed 10 of 10 (7.0 to 9.6 seconds). It also passed in the full 4b run. C-run.

It is still a real gate defect. A gate that goes red under load teaches people to re-run until green. It ranks as candidate 7.

**Per-binary `test result:` lines from run 4b, in cargo order:**

| binary | passed | ignored | binary | passed | ignored |
|---|---|---|---|---|---|
| pii_abi lib | 45 | 0 | pii_proto lib | 7 | 0 |
| pii_channels lib | 7 | 0 | wire_roundtrip | 11 | 0 |
| pii (cli) bin | 29 | 0 | pii_server lib | 46 | 0 |
| pii_core lib | 23 | 0 | agent_line_gate | 1 | 0 |
| pii_evolve lib | 0 | 0 | approvals | 2 | 0 |
| pii_evolve bin | 0 | 0 | boss_watch | 8 | 0 |
| launcher | 3 | 0 | cancellation_brake | 5 | 0 |
| state_tests | 2 | 0 | channels | 2 | 0 |
| pii_graph lib | 38 | 0 | fail_port | 1 | 0 |
| brake | 1 | 0 | failover_graph | 3 | 0 |
| diamond | 5 | 0 | handshake | 4 | 0 |
| examples | 4 | 0 | latency | 1 | 0 |
| journal | 6 | 0 | memory_rpc | 1 | 0 |
| loops | 2 | 0 | model_routing | 6 | 0 |
| resume | 3 | 0 | peercred | 2 | 0 |
| pii_http_fetch lib | 5 | 0 | reattach | 1 | 0 |
| fetch | 6 | 0 | scheduler | 3 | 0 |
| pii_mcp bin | 3 | 0 | session | 7 | 0 |
| mcp_eof | 1 | 0 | slice_e_live | 4 | 0 |
| pii_memory lib | 4 | 0 | slice_f_promotion | 4 | 0 |
| consolidate | 6 | 0 | spawn_node | 2 | 0 |
| lanes | 8 | 0 | pii_store lib | 27 | 0 |
| persistence | 1 | 0 | mem_store_contract | 7 | 0 |
| pii_model lib | 24 | 2 | store_contract | 5 | 0 |
| ollama_http | 5 | 0 | pii_tools lib | 31 | 0 |
| real_granite | 0 | 8 | pii_tui lib | 5 | 0 |
| pii_net lib | 10 | 0 | render | 5 | 0 |
| accept_pool | 3 | 0 | pii_uds lib | 2 | 0 |
| egress | 12 | 0 | pii_web bin | 11 | 0 |
| http_codec | 5 | 0 | pii_worker lib | 2 | 0 |
| | | | supervisor | 9 | 0 |
| | | | xtask lib | 8 | 0 |
| | | | xtask bin | 42 | 0 |

Every line read `ok. N passed; 0 failed`. The 10 ignored tests are the real-model tests that load a 2.2 GB GGUF file, unchanged. Since 2026-09-18 the total grew from 505 to 536, which is the tests added by specs 62-69. C-run.

## 2. Are the two 2026-09-18 gaps closed?

| gap | state today | evidence |
|---|---|---|
| Cargo never ran (brief items 3 and 5) | **Closed.** The September run 2 executed all gates. This run re-executed all six verbatim, with raw output above. | C-run |
| Only 4 of 20 crates analysed | **Closed for reading.** The September run 2 covered the other 15 crates. This run added a second best-practice pass across all 20 crates plus xtask, in section 7. | C-read and I |
| Residue: did the tests prove anything? | **Was open, now measured.** Seven September fixes were mutation-tested. Four are only partly guarded (section 3). The September reachability claim "passes today and proves the gate would have caught B1" was never tested by mutation, and it is false (section 6, Decision 5). | C-delegated and C-run |

## 3. Specs 62-69 verified by behaviour, not by claim

Method: export HEAD to scratch, revert each fix in isolation, and run its targeted tests. A fix counts as guarded only if a test turns red. Every baseline passed unmutated. Logs and scripts are in `/projects/_tmp/pii-review/mut-H6eG/`. All rows are C-delegated except spec-69, which is C-run.

| spec | fix | mutation | result | verdict |
|---|---|---|---|---|
| 62 | worktree signature validation | validator disabled | 2 tests red (`rejects_too_short_signature`, `rejects_non_hex_signature`) | guarded |
| 62 | post-join containment check | call site disabled (`if false && ...`) | **10 of 10 green** | **unguarded at the call site.** Only the helper is unit-tested. |
| 62 | both checks | both off | 4 red | guarded as a pair. The traversal tests pass with either check alone, so each check masks the other. |
| 63 | pii-web `q` key | back to `"query"` | `api_memory_route_reaches_daemon_successfully` red | guarded |
| 64 | restart backoff | sleep removed | `respawn_honors_configured_backoff` red | guarded |
| 65 | `layer_of` reads the stored layer | constant 0 | `consolidate_same_layer_only_true_blocks_cross_layer_merge` red | guarded |
| 65 | layer persisted on write | `MEM_LAYER` insert dropped | **green on a used dev tree**; red only after moving stale `crates/pii-memory/_tmp/phase3/cons-*.redb` aside. pii-store tests stay green. | **partly guarded.** The tests are not hermetic, and no pii-store test reads a layer back. |
| 66 | `PiiError::store_op` | op and resource swapped | only the constructor test red; all pii-store tests green | **partly guarded.** No test asserts the message at any of the converted call sites. |
| 67 | line-counter attribute stack | fix reverted | `handles_attribute_stack_before_cfg_test_body` red. `failover.rs` now counts 195 effective lines, not 359. | guarded (the second regression test, `handles_bare_cfg_test_fn`, guards nothing) |
| 67 | lint-files in gate; `std::println` path; `xtask ci` delegates to `quality-gate.sh` | source read | `quality-gate.sh:68-75`, `clippy.toml:22`, `xtask/src/main.rs:94`, `scripts/mvp-verify.sh:47` | done (C-read; clippy log shows zero warnings, C-run) |
| 68 | `cmd_run` exit codes, `cmd_soak` status match | codes swapped, strings renamed | red | guarded |
| 68 | `cmd_sandbox_exec` `& 0xff` | changed to `& 0x7f`, or mask dropped | **29 of 29 green** | **unguarded.** `as u8` already truncates, so the mask is dead code. The "300 to 44" case is vacuous, and no case uses an exit code of 128 or more. |
| 68 | chmod failure logged; `200-OK` literal | source read | `cmd_daemon.rs:252-255` prints a warning; `pii-http-fetch/src/lib.rs:96` reads `-> status={status}` | done (C-read) |
| 69 | `lint-files --report N` | run | prints the top N with effective and raw counts; max is 360 | done (C-run) |

## 4. 031/032 findings: fixed, standing, or wrong

| finding | status | evidence |
|---|---|---|
| 032 F1: the 450 list used the wrong metric | **Correct and now encoded.** 031 was corrected by spec-69, and lint-files reports a maximum of 360. | C-run |
| 032 F2: lint-files missing from the pidag gate | **Fixed** (spec-67) | `quality-gate.sh:68`, C-read |
| 032 F3: counter attribute quirk | **Fixed and guarded** | section 3 |
| 032 F4: F-11 pass 2 blocked on a constructor | **Fixed** for `heal.rs` and `session.rs`. About 174 `PiiError::Internal(` sites remain in pii-store (`lib.rs` 64, `mem/read.rs` 31, `mem/write.rs` 20, `timer.rs` 16, `channel.rs` 15, `approval.rs` 14, `mem/mod.rs` 10). | I |
| 032 F5 to F10 | **Fixed** (specs 62-64, 65, 68). Guard gaps are listed in section 3. | C-delegated and C-read |
| 032 observation (a).1 (`println` rule) | **Fixed** | C-run |
| 032 observation (a).2 (zero doctests) | **Still stands** (0 of 17) | C-run |
| 032 observation (a).3 (lint-files silent) | **Fixed.** It now prints a summary line. | C-run |
| 032 claim: "No hardcoded `/tmp` or `/projects/` paths remain in production code" | **Wrong as stated.** It held only for the 13 crates read. xtask has 15 `/projects/pi-improved` literals (section 6). | C-run |
| 032 claim: "pii-net ... the only lenience is consuming `\r` without confirming `\n`" | **Understated.** That lenience is a panic plus spurious errors on the model streaming path (section 6, D1). | C-run |
| 032 F10: "Defence in depth holds because the socket dir is 0700" | **Weaker than stated.** The 0700 chmod in `engine/uds.rs:41` is also discarded. | I |
| 031: "pii-web (2 tests)" | **Out of date.** It now has 11, including one route test. | C-run |
| 031: "pii-cli command files untested" | **Partly fixed**, see section 5 | C-run |
| 031 deferred items (F-11 sweep, harness graph, modularity, DAG unification, research reorg) | Untouched by the intervening work, apart from F-11 pass 2. They are ruled on in section 6. | C-read |

## 5. Previously skipped crates: the two named checks

**pii-cli command files.** Counts are `#[cfg(test)]`, `#[test]` and `#[tokio::test]`, from a grep on HEAD. C-delegated, consistent with the binary's 29 passing tests (C-run).

| file | cfg(test) | test | tokio::test | true state |
|---|---|---|---|---|
| cmd_ctl.rs | 0 | 0 | 0 | untested; thin argv-to-JSON glue, low risk |
| cmd_daemon.rs | 1 | 2 | 0 | exit-code mapping and `self_uid` tested. The accept loop and shutdown are untested (see 7.E8). |
| cmd_run.rs | 1 | 5 | 0 | `parse`, `report` and `root_ctx` tested and guarded |
| cmd_sandbox_exec.rs | 1 | 1 | 0 | one test, which partly guards nothing (section 3) |
| cmd_soak.rs | 1 | 1 | 0 | status matching guarded, but it still matches strings instead of `WireRunStatus` and has no cancelled arm (I) |
| cmd_sys.rs | 1 | 1 | 0 | `run_ls` filter tested |
| cmd_tui.rs | 0 | 0 | 0 | untested; filesystem-coupled connect-mode resolution |
| cmd_worker_exec.rs | 0 | 0 | 0 | covered indirectly, since `pii-worker/tests/supervisor.rs` spawns it as a real subprocess |

**pii-web.** It now has 11 tests, one of which drives `/api/memory` against a fake daemon socket, and that test is proven to guard the spec-63 fix. The coverage is adequate for the one route that broke and inadequate for the rest. I-level items remain:
- The error bodies are built with `format!("{{\"error\":\"{e}\"}}")` at 8 sites (`main.rs:82` and others), which produces invalid JSON when the message contains a quote.
- There is no timeout on the daemon call.
- The `tower-http` dependency, including its `cors` feature, is still unused.

## 6. New defects (all proven or read by me unless marked)

| # | sev | location | defect | demonstrating command |
|---|---|---|---|---|
| D1 | **HIGH** | `crates/pii-net/src/service/dechunk.rs:45,54-56`, caller `service/http.rs:91,112` | The completeness check measures from the size line, not from `data_start`. A read that ends after a chunk's `\r` advances `pos` past the buffer, and `pending.drain(..consumed)` then panics. Other split points produce spurious `bad chunk size` or `truncated chunk` errors. The path is live: `pii-model/src/ollama.rs:201` streams every model turn through `request_streaming`, which calls `dechunk_step`. | Demo test in `mut-H6eG/src` (`dechunk.rs` `review_demo`) splitting `5\r\nhello\r\n0\r\n\r\n` at every offset: **5 of 16 splits fail**. The split at offset 9 gives `consumed 10 > pending 9 (drain would panic)`; offsets 7, 8, 13 and 14 give errors. `cargo test -p pii-net --lib review_demo` exits 101. C-run |
| D2 | **HIGH** | `crates/pii-tools/src/bash.rs:115` | `s.truncate(MAX_OUTPUT)` runs on a lossily decoded `String`. A command printing more than 64 KiB of multibyte text panics when the cut lands mid-character. Line 106 does it correctly, on bytes. | Demo test (`x` followed by 40,000 `é`), `cargo test -p pii-tools --lib review_demo`: `panicked at crates/pii-tools/src/bash.rs:115:15`. C-run |
| D3 | **HIGH (gate)** | `xtask/src/reachability.rs:44-57,132,139-145` | The gate is blind to both classes it exists for. Entry 1 searches `crates/pii-model/src`, the declaring crate, and is satisfied by pii-model's own internal calls (`lib.rs:242,266`). Entry 2's needle `.invoke(` matches any method of that name. The doc promises `line_is_test` exclusion of `#[cfg(test)]` modules, but no such function exists. Its test hardcodes `/projects/pi-improved/_tmp/...` (line 171). It is not in `quality-gate.sh`. | Scratch copy with both real callers removed (`pii-server/src/lib.rs:203` and `node_kinds/nodes/worker.rs:38`): `reachability-check` **exit 0**. With the seed corrected to `pii_model::resolve_with_transport(` in `crates/pii-server/src` and `sup.invoke(`: mutated tree **exit 1** naming both, HEAD tree **exit 0**. Logs in `/projects/_tmp/pii-review/reach-zu2N/`. C-run |
| D4 | MED-HIGH | `crates/pii-server/src/engine/runs.rs:171` (also `:151-158`) | Resume fails open. A store error in `stored_run_scope_root(..).ok().flatten()` resumes a confined heal or promotion run with workspace-wide write access, which the comment at `:166-176` says must never happen. A failed graph lookup gives an unlimited budget (`map_or(u64::MAX, ...)`). | Read, C-read. A fault-injected store test is the proof to add. |
| D5 | MED-HIGH | `crates/pii-graph/src/journal.rs:233-240` | `flush` drains `pending` before `append_events` succeeds, so a store error loses those events. Callers discard the error: `journal.rs:165` `let _ = self.flush();`, `run/engine.rs:332`, `sched/dispatch.rs:324`. | C-read (drain order). Caller list is I. |
| D6 | MEDIUM | `crates/pii-server/src/daemon/mod.rs:199`, `engine/bus.rs:27` | The reactive loop that arms timers and approvals on `RunSuspended` and `RunFinished` ignores `RecvError::Lagged`. It shares the 4096-slot broadcast channel with per-token `AssistantDelta` events (`agent/turn.rs:102`). A long streamed turn can make a suspended run never resume. | C-read. A test flooding more than 4096 deltas around a suspend is the proof to add. |
| D7 | MEDIUM | `xtask/src/{heal_prune,heal_seed,heal_collect,rss_gate,memory_eval,promote_build,doc_drift,main}.rs` | 15 hardcoded `/projects/pi-improved` literals (spec-61 covered only `crates/`). xtask commands in any clone or worktree act on this checkout, which is the wrong tree, including heal worktrees and promotion builds. `workspace_root()` already exists in `xtask/src/main.rs`. | `grep -rn '/projects/pi-improved' crates xtask --include=*.rs \| grep -v '//'` prints 15 lines. C-run |
| D8 | MEDIUM | `crates/pii-memory/tests/*`, `crates/pii-memory/_tmp/phase3/` | Tests reuse persistent redb files under a gitignored `_tmp`, so a regression can pass on any machine that ran the suite before (section 3, spec-65). | C-delegated |
| D9 | LOW-MED | `crates/pii-server/tests/boss_watch.rs:205` | A 10-second wall-clock deadline makes the gate red under load. | Section 1. C-run |
| D10 | LOW | `crates/pii-server/src/lib.rs:440` | `graph_dir.join(format!("{name}.toml"))` does not reject `..` in an RPC graph name. Only same-uid peers reach it, and `graph.apply` already stores arbitrary TOML, so it grants nothing new. Fix it as hygiene. | C-read |

Additional HIGH and MEDIUM items from the best-practice pass are in section 7, marked I where not re-checked.

## 7. Rust best-practices refinement (distinct, ranked)

`unsafe` stays forbidden and nothing here proposes it. Each item gives severity, location, the change, and the gate that would prove it.

**A. Async correctness**

| # | sev | location | change | proving gate |
|---|---|---|---|---|
| A1 | HIGH | `pii-server/src/engine/conn.rs:170` calls the synchronous `dispatch` on a tokio worker. That runs `memory.consolidate`/`search` (redb), `worker.reload` (up to about 6 seconds of sleeps and waits, `pii-worker/src/control.rs:228`), `run.trace` journal reads, and `session.prompt` (about 4 fsync'd transactions). The daemon has 4 workers. (I, C-read for the call shape) | wrap heavy methods in `spawn_blocking` | current_thread heartbeat test per method, the pattern `daemon/router.rs:265` already uses |
| A2 | HIGH | `pii-proto/src/client.rs:72,110` keeps its own `events_tx`, so `events().recv()` never sees `Closed` while the client lives. `cmd_oneshot.rs:260` and `pii-tui/driver.rs:70` can hang after the daemon dies. (C-read for the retained sender, I for the hang) | drop the sender on reader exit and set a closed flag that `call` checks | duplex-close test where `recv` and `call` must return within 1 second |
| A3 | HIGH | `pii-core/src/childproc.rs:289-310` joins the output-drain threads after `try_wait`, and uses unbounded `read_to_end`. `sh -c 'sleep 600 &'` outlives the timeout, and `yes` grows memory without bound. (I) | kill the process group before joining, read through `.take(cap)` | 1-second-timeout tests with a background sleeper and with `yes` |
| A4 | MEDIUM | `pii-server/src/engine/conn.rs:114,127` uses an unbounded `mpsc` writer channel and an uncapped `lines()` reader, although `max_frame_bytes` of 1 MiB is advertised. `subscribe.rs:104` drains the bounded 1024 queue into the unbounded one. (I) | bounded channel that closes the connection when full; capped `read_until` | add `tokio::sync::mpsc::unbounded_channel` to `clippy.toml` `disallowed-methods`, plus a 2 MiB-line test |
| A5 | MEDIUM | `pii-cli/src/cmd_daemon.rs:263` `let Ok(..) = listener.accept().await else { continue; }` spins on EMFILE, and per-connection tasks are uncapped. `:158` drops the runtime without `shutdown_timeout`. (I) | back off and add a semaphore; `rt.shutdown_timeout(2s)` | accept-error injection test; SIGTERM-with-run-in-flight test |
| A6 | MEDIUM | `pii-model/src/llamacpp.rs:143-196` never checks cancellation (`let _ = ctx;`), and holds the provider lock for the whole generation. (I) | `ctx.check()?` per token | cancelled-context test with `max_tokens = 0` |
| A7 | MEDIUM | `pii-server/src/daemon/scheduler.rs:61,87,91` spins on a store error without awaiting. `let _ = spawn_resume(..)` loses a wakeup after `take_due` has already deleted the timer. (I; the `let _ =` at `:91` is C-read) | back off and yield on `Err`; re-arm if resume fails | `start_paused` test with a failing timer wheel |
| A8 | MEDIUM | `pii-worker/src/supervisor.rs:245,251` calls `death(None)` with no pid check, so a stale reader can kill a freshly reloaded child. `control.rs:83-88` leaks the slot when `send` fails. (I) | `death(pid, code)` returns early on a pid mismatch; remove the slot on `send` error | reload test with a grandchild holding stdout; slot-leak test |

**B. Error taxonomy**

No `anyhow` and no `Box<dyn Error>` anywhere (I). The real problem is silent discarding.

| # | sev | location | change | gate |
|---|---|---|---|---|
| B1 | MEDIUM | about 20 `let _ =` and `.ok()` sites that lose state: `daemon/mod.rs:336` (approval never created, so the run is suspended forever), `session_durability.rs:315` (zombie run), `agent/turn.rs:186` (MEMORIZE fact lost), `router.rs:107,110` (duplicate runs), plus D4, D5, D6 and A7 above (I, plus the C-read items cited there) | propagate the error or record it as an event | enable `clippy::let_underscore_must_use`, which turns this whole class into a compile error |
| B2 | MEDIUM | store reads mapping errors to "absent" or zero: `pii-store/src/mem/write.rs:55,125`, `mem/read.rs:24,70,85,123,265`, `session_durability.rs:76,102,109` (`to_vec(..).unwrap_or_default()` writes an empty blob over valid data) (I) | return `Result` | fault-injection tests using the existing `set_fail_session_writes_for_test` seam |
| B3 | LOW | `RpcError` lacks `Display`/`Error` (5 hand-formatted sites); `ClientError::Io(String)` and `TuiError(String)` drop their source (I) | implement `Display`/`Error`; keep `#[source]` | `clippy::missing_errors_doc` is already on; add a test asserting the source chain |
| B4 | tracked | F-11: about 174 `Internal(` sites left in pii-store, 13 in pii-model, 7 in pii-server, 5 in pii-graph; `sched/state.rs:184` is a genuine `Result<(), String>` (I) | continue passes per file with `store_op` | `grep -c 'PiiError::Internal(' <file>` reaches 0 per pass |

**C. Hot-path allocation**

| # | sev | location | change | gate |
|---|---|---|---|---|
| C1 | MEDIUM | each node output is deep-cloned 6 or more times per event: `journal.rs:186,192`, `sched/dispatch.rs:305,360`, `sched/state.rs:244` (once per outgoing edge), `bus.rs:118`, plus one per subscriber (I) | `Arc<Value>` / `Arc<RunEvent>` in `SeqEvent` and the journal | counting-allocator test with a 1 MiB output; `xtask bench-gate` case |
| C2 | MEDIUM | `sched/dispatch.rs:155` re-serialises the whole blackboard to `Value` per node per tick; a failure silently becomes `Null` (I) | build once per tick and share; propagate the error | 1000-iteration loop bench |
| C3 | MEDIUM | `pii-memory/src/bm25.rs:198,200` re-reads and re-tokenizes the document for every posting, with an O(P²) `iter_mut().find` accumulator (I) | store document length at write time; `HashMap` accumulator | 10k-document bench |
| C4 | MEDIUM | `engine/session_durability.rs:99` clones the full transcript on every append but uses it only every 32nd; `lib.rs:357,362` clones twice more; `agent/turn.rs:79` copies the transcript each turn (I) | clone inside the snapshot branch; borrowing wire structs | allocations-per-append test |
| C5 | MEDIUM | `pii-model/src/ollama.rs:202` applies `from_utf8_lossy` per chunk, corrupting multibyte characters split across chunks into U+FFFD; `:162,254,296` assigns `call-{name}` ids that collide (I) | byte buffer with `encoding_rs`, as `llamacpp.rs:142` does; index-based ids | split-`é` test; two-calls-same-tool test |
| C6 | LOW | `pii-proto/src/lib.rs:188-238` uses `.cloned()` per frame where `Value::take` fits; per-call unbounded channel where `oneshot` fits (I) | as stated | micro-bench in `xtask bench-gate` |

**D. Trait design and the 450/traits-first gate in practice**

The traits are sound. `Tool` has 13 impls, `Observer` 5, `ModelProvider`, `NodeFactory`, `GraphNode` and `GateRunner` each have production plus test doubles, and every trait object is justified (I). No change.

The 450 gate works: max 360 effective lines (C-run). It has one side effect worth fixing. Two splits made only to satisfy the line gate created module cycles:
- `daemon/methods.rs:4` was "split out of `engine::dispatch` to keep that file under" the limit. It now forms `engine/dispatch.rs:130-142` to `daemon::methods` to `engine::dispatch` (C-read, see Decision 2).
- Rule: a size split must move code to where its dependencies point. Gate: `grep -rn 'crate::daemon' crates/pii-server/src/engine | grep -v '//'` should print nothing.

**E. API design**

| # | sev | location | change | gate |
|---|---|---|---|---|
| E1 | LOW-MED | stringly-typed status and kinds: `cmd_soak.rs:176` matches `"succeeded"`/`"failed"` although `WireRunStatus` exists; `subscribe.rs:201` uses `kinds: &[String]`; `WireMessage.role: String` (`pii-server/src/lib.rs:359`) duplicates `pii_model::Role`; worker ops are strings (I, plus C-read for `cmd_soak`) | enums, `EventKind`, `ModelSpec` | a test that a typo'd kind is rejected |
| E2 | LOW | `RunStatus` to `WireRunStatus` mapping hand-written three times (`dispatch.rs:259,299,381`); `client_info()` copied 7 times; uid parsing duplicated in `cmd_daemon.rs:342` and `pii-uds/lib.rs:70`, falling back to uid 0 (I) | `From` impls, one shared helper, `nix::unistd::Uid::current()` | `clippy.toml` `disallowed-methods` on the old call sites |
| E3 | LOW | `pub mod` surfaces in pii-graph (`nodes`, `run_helpers`, `sched::*`) and test hooks on production `Store` (`set_fail_session_writes_for_test`) (I) | move hooks behind a `testkit` feature | `cargo public-api` diff (tool not installed here; NOT VERIFIED) |

**F. Dependency hygiene**

| # | sev | finding | gate |
|---|---|---|---|
| F1 | LOW-MED | `llama-cpp-2` is an unconditional dependency of `pii-model`, so every crate that depends on it (pii-tools, pii-server, pii-cli, xtask transitively) builds vendored llama.cpp C++ (C-read: `pii-model/Cargo.toml:16`, `pii-tools/Cargo.toml:13`). Gate it behind a `llamacpp` feature. | `cargo tree -e features -i llama-cpp-2`, plus build time of `-p pii-tools` |
| F2 | LOW | likely unused declared dependencies (I, may include test-only false positives): `thiserror` in about 9 crates, `tower-http` in pii-web, `blake3` in pii-graph, `pii-core` in pii-store, tokio `test-util`/`process` in pii-server `[dependencies]`; `encoding_rs = "0.8"` not using `workspace = true` | `RUSTFLAGS=-Wunused-crate-dependencies cargo check --workspace`, or install `cargo-machete` (not installed here) |
| F3 | LOW | duplicate major versions in `Cargo.lock`: `syn` 1/2/3 (C-run), `nix` 0.29 and 0.31 (I) | `clippy::multiple_crate_versions` is already in the `cargo` group as warn; check why it is not surfacing (NOT VERIFIED) |
| F4 | LOW | lockfile committed and in sync; `ratatui = "=0.30.2"` pin now carries a reason comment (spec-68 `Cargo.toml` change, I) | none needed |

## 8. The eight decisions

### Decision 1: coding-harness graph shape. APPROVE with two amendments.

| evidence | label |
|---|---|
| An agent node's model is fixed when the node is built. `resolve_model(cfg, ..)` is called in the factory `build` (`pii-server/src/node_kinds/agent/mod.rs:86-103,197`), so one node cannot switch model per iteration. | C-read |
| `graphs/starter/failover.toml` already expresses escalation as two agent nodes chained by a route, plus a documented `loop_budget` N allows N-1 quirk (`:20-28`). | C-read |
| `validate-exit-criteria/run.sh` lives at `~/.pi/agent/skills/`, outside the repo. | C-run (`ls`) |
| The route-on-payload constraint (030:346-362): routed findings must be a successful `Outcome` with a payload. This is consistent with failover and with the tool node reporting `exit_code`. | C-read |

Ruling:
- Fork `verify-fix.toml`.
- **Amendment 1:** model escalation means two `agent` nodes (free, then paid) joined by a `route` on the test result, copying `failover.toml`. It needs no engine change and no per-iteration model switching.
- **Amendment 2:** multi-criteria validation is one `tool` node running an in-repo script (for example `scripts/validate-exit-criteria.sh`, vendored). It emits `{pass, fail, total}` as a success payload that the route reads. Do not reference the `~/.pi` path; that would add a 16th host-path dependency to the class in D7.
- Keep `heal-fix`'s worktree confinement and human gate whenever the harness writes anything that can leave a disposable worktree. Spec-62's validator is now the confinement primitive, so close its call-site test gap first (candidate 6).

**Confirmed: routed findings are success payloads, not `Err`.**

What would change my mind: if the two-node pattern hits the `loop_budget` accounting quirk in a way that cannot be configured around, a per-iteration `model` expression on the agent node becomes the cheaper path.

### Decision 2: F-13 as a module reorganisation. CONFIRMED as the right call, with one follow-up.

| evidence | label |
|---|---|
| Cross-module references inside pii-server: engine to node_kinds in 2 files, engine to daemon in 2, node_kinds to engine in 6, node_kinds to daemon in 0, daemon to engine in 3, daemon to node_kinds in 0. | C-run |
| The engine-to-daemon edge is exactly `engine/dispatch.rs:130-142`, which routes 13 methods into `daemon::methods`. `daemon/methods.rs:4` says it was split out of dispatch for the line limit. The other direction is `daemon/scheduler.rs:91`, `approvals.rs:33` and `methods.rs:19,331`. | C-read |
| No harness requirement in Decision 1 needs crate-level isolation. The harness is TOML plus existing node kinds. | C-read |

Ruling: a 3-crate split would first have to break a cycle that exists only because of a line-gate split. That is not worth a crate boundary today.
- **Follow-up:** move `daemon/methods.rs` to `engine/methods.rs`. It is pure relocation with zero behaviour change.
- Exit criterion: `grep -rn 'crate::daemon' crates/pii-server/src/engine | grep -v '//'` prints nothing.

What would change my mind: a second binary needing the engine without the daemon, or measured incremental rebuild time of pii-server becoming the bottleneck of the spec loop.

### Decision 3: modularity ("microservices in a single binary"). TOO EARLY. No design document yet.

| evidence | label |
|---|---|
| The in-process versus isolated mechanism already exists: the `worker` node kind calls `sup.invoke` (`node_kinds/nodes/worker.rs:38`) over pii-worker's stdio protocol. | C-read |
| That mechanism is not hardened enough to be the uniform module interface. Restart backoff was dead code until spec-64. A stale reader can kill a reloaded child (A8). A failed `send` leaks a slot. | C-delegated and I |

Ruling: generalising it into "every module is swappable" means designing on top of the least-proven seam. Harden pii-worker (A8) first.

What would change my mind: the first module that must leave the process. The best candidate is in-process llama.cpp. It ignores cancellation and holds a global lock for the whole generation (A6). If that cannot be fixed in-process, moving llama.cpp behind pii-worker is the prototype, and its write-up becomes the design document.

### Decision 4: Chromecast. DEFER as pii work. If built, use a named-device registry only, with no change to the SSRF boundary.

| evidence | label |
|---|---|
| `pii-net/src/authorize.rs:202-208` returns `BlockedClass` for `AddrClass::Multicast` unconditionally, with no grant field to relax it. | C-read |
| 038 shows control (TLS on port 8009, private-IP grant) works today. | C-read via the docs reviewer (038 verdict CURRENT) and I |
| 038's own open question: a graph has nothing to route on from a cast outcome. | C-read |

Ruling: auto-discovery is not worth widening the one boundary every egress passes through. Devices named in config give the whole feature. If IP churn makes a static registry painful, run mDNS discovery as an out-of-band helper, either a separate process or a user-run command, that writes the registry. It must never be a `pii-net` capability. Because casting gives a graph nothing to route on, a standalone daemon fits better than a pii crate.

What would change my mind: a graph that routes on cast results, such as playback failure triggering a fallback to TTS on another device. That would make it a real node kind.

### Decision 5: upgrade `reachability-check` to a call graph. APPROVE the goal, REJECT the proposed means as the first step.

| evidence | label |
|---|---|
| The current gate passes (exit 0) with both real callers removed. | C-run (D3) |
| graphify's graph (`graphify-out/graph.json`, built 2026-09-19, code unchanged since) has **no incoming `calls` edge** to `resolve_with_transport` from pii-server, none to `Supervisor::invoke` from `worker.rs`, and none to `provider_from_env` from `pii-model/src/lib.rs`. The path-qualified cross-crate call and the method call on a local binding are both missed. A gate on that graph would go red on a correct tree. | C-run (Python over `graph.json`) |
| A two-string seed correction makes the gate fail on the mutated tree and pass on HEAD. | C-run |
| `syn` is already in `Cargo.lock` in three versions. | C-run |

Ruling, in order:
1. Correct the seed. Implement real `#[cfg(test)]` and comment exclusion by reusing `lint_files`' test-body skipping. Delete the phantom `line_is_test` doc. Remove the hardcoded test path. Add the check to `quality-gate.sh`.
2. Only if the living list grows past about 10 entries, or a false pass recurs, replace the matching with a `syn` visitor inside xtask that resolves path-qualified calls and ignores strings and comments.
3. Method calls on receivers need type information. Neither tree-sitter nor `syn` has it, so for that class the strongest oracle is a behavioural test, such as the existing `supervisor.rs` tests driven through a graph `worker` node.

What would change my mind: a `syn` prototype catching, on this repo's history, a miss the corrected string seed does not catch.

### Decision 6: a native code-knowledge-graph service. NO, not now. The veto holds.

The only gate a code graph was proposed to strengthen is `reachability-check`. Measured here, a two-string fix made that gate strictly stronger with no graph, while the existing graph would have made it weaker by producing false reds (Decision 5 evidence, C-run).
- (a) Shape 2 over Shape 1. Shape 1, wrapping graphify, is now measurably wrong, not just stylistically foreign.
- (b) If ever built: a parser in xtask, gate-time only. Not in pii-memory or pii-store.
- (c) Rust-only, `syn` before tree-sitter, because `syn` is already compiled.
- (d) Gate-time only.

What would change my mind: a gate that fails today on a real regression the test suite missed, which only graph structure can catch, demonstrated by replaying this repo's spec commits.

### Decision 7: should the harness consume the graph? NO. Decouple it.

Decision 1 is ready and should not wait. Of the three proposed uses:
- Pre-edit orientation is a context convenience, not a gate, so the veto excludes it.
- Blast radius is the only gate-strengthening use. It needs the route-payload pattern and evidence that tests miss what it catches. Revisit after the harness has run about 10 specs and its misses are recorded.
- Tool-set narrowing does not need a graph. `pii-mcp/src/main.rs:40` registering `ToolSet::builtin` wholesale is fixed by a static per-graph allowlist in config.

On Track B against DAG unification: a code graph is a data structure, not an execution plan. It is orthogonal, not a third format.

### Decision 8: `_docs/06-research/` reorganisation. Keep flat by number. No subfolders. Fix the index discipline instead.

The consolidation commits (`33bf2fc`, `cd43474`) already chose flat numbering with canonicals and `99-archive`. That is right for chronological research. The remaining problems are discipline, not layout (I, with the 041 collision C-run):
- Three files claim number 041: the brief, the appendix, and this report's planned filename.
- `000-TOPIC-MAP.md` is still a plan, with "Nothing has been deleted or moved yet" and a "Still to do" list that is already done.
- No pointer stubs exist at the merged-away paths, although the map promised them.
- INDEX and TOC copy the research list instead of linking it.

Rules to adopt:
1. One number per document.
2. A status banner is mandatory.
3. INDEX and TOC link to `000-TOPIC-MAP.md` instead of copying it.
4. File this report as `043-opus-full-review-2026-10-06.md`, not 041.

### pidag as a constrained subset of pii's schema: CONFIRMED (option a), with one amendment.

pii's port routing strictly contains pidag's pass/fail `depends_on`, and `failover.toml` and `heal-fix.toml` depend on that expressiveness. The amendment concerns where the shared schema lives:
- `pii-graph/src/spec.rs` is 300 lines and imports only `pii_abi::{NodeId, PiiError, PortName}` and serde (C-read).
- So extract it into a small schema crate that pidag can depend on without pulling in pii's 19-crate runtime. pidag is then a validation profile over that schema (only `pass` and `fail` ports, only its own node kinds).
- The trigger for the identical-format phase is the Decision 1 harness. When it exists, both projects encode the same implement, validate, gate loop.

## 9. Documentation review (new contract)

Method: two parallel read-only passes covered every area, with about 85 concrete claims checked by command. I re-checked six of them myself: `HANDOFF.md:188` and `:237` "Nothing is committed" and "No commits"; `AGENTS.md:51` "18 crates"; `.pidag/config.toml:12` `max_iterations = 2` against `pidag-way-of-work.md:91` "4"; `Cargo.toml:56` against cited `:54`; and 030's triple 037 bullets at `:140-145` plus dead `035/036` at `:485`. All six held (C-run). The rest are I.

**Per-area verdicts**

| area | verdict | key reason |
|---|---|---|
| AGENTS.md, CLAUDE.md, .claude/CLAUDE.md | MINOR DRIFT | "18 crates" (`AGENTS.md:51`, actually 19 plus xtask); dead `06-research/001-003-*.md` (`:8`); `Cargo.toml:54` should be `:56`. graphify rules are duplicated and already diverging between CLAUDE.md and AGENTS.md. |
| HANDOFF.md | STALE in its lower half | Top block and 62-69 accurate. `:188` and `:237` say nothing is committed; `:210-215` mark F-11 to F-20 "NOT STARTED"; specs 57-61 missing despite "43-61 COMPLETE" (`:16-18`); `:32` points at moved `06-research/033-*`; `_tmp/gate-spec*.log` evidence pointers dangle. |
| _docs/INDEX.md, TOC.md | STALE | INDEX layout says "(future) src/"; "18 crates, 40+ specs"; TOC marks 05-operations and 99-archive "(empty)"; both copy the research list near-verbatim; 023 and 033 filed under the wrong headings. |
| 01-overview/001-pii-vision.md | STALE | "~14 crates", "2,000-line gate" (actually 450 effective); link text names `001-...` over a link to `002-...`; no "design intent" banner. |
| 02-guides | MINOR DRIFT to CONTRADICTED | `pidag-way-of-work.md:91` says 4 iterations, config says 2; gate shape predates the 5-step gate; `pii-mcp-bridge.md:23` documents a `pii mcp` subcommand that does not exist (it is the `pii-mcp` binary); 023 says spec 36 is "being implemented" though it landed. |
| 05-operations/033 | STALE | "NOT committed"; landed in `2e47219`. Its `doc.rs` offset-bug note is still open. |
| manuals (.tex) | MINOR DRIFT | method table has 32 of 34 (`run.resume` and `run.trace` missing); crate table lacks `pii-uds` and `pii-http-fetch`; cites archived `008-pii-evolve.md`. |
| 06-research | mixed (per-file table below) | |
| 99-archive | FINE-ARCHIVAL | Merge provenance comments name the canonical instead of the original in 10 places (002:2336,3699; 012:208,430; 014:361,424; 026:312,837; 037:229,503). |

**06-research per-file verdicts**

| verdict | files |
|---|---|
| CURRENT | 016, 018, 019, 020, 021, 022, 034, 038, 040, 042 |
| STALE-STATUS | 012 (no banner; specs 30-34 done), 013, 015 ("DRAFT", but llama.cpp is implemented), 024 (caveats fixed in `29ea8aa`), 025 ("No implementation", Track A landed), 029, 030, 031 (`:79-81`, `:148` contradict its own correction), 032 ("nothing committed"), README, 000-TOPIC-MAP |
| CONTRADICTED | 000-CANDIDATES-INDEX (025 "no spec and no code"), 014 (calls 015 settled) |
| DUPLICATE | 037 (and 030 `:247-281` restating it), 039 §7 (restates 040) |
| FINE-ARCHIVAL | 002, 005, 006, 007, 026, 027, 028 |

**Highest-value corrections**, one line each, not rewrites:

1. HANDOFF: put a "historical snapshot (2026-09-16), superseded by CURRENT STATE" banner above `:186`; add one line for specs 57-61; repoint `:32` to `_docs/05-operations/033-...`.
2. AGENTS.md: `:51` "19 crates + xtask"; `:8` to `002-pii-architecture-plan-pass2.md (carries passes 1-3)`; `:~58` to `Cargo.toml:56`.
3. 030: collapse `:140-145` to one 037 bullet; replace `:485` with 037; update the "28 files" decision text; add a pointer to this report.
4. 000-TOPIC-MAP: replace the plan text with the current map; mark "Still to do" done; resolve the 041 collision (appendix to 043-, this report to 043/044).
5. 025, 024, 015, 012, 032, 033: one status banner each, citing the landing commit.
6. INDEX and TOC: replace the copied research lists with a link to 000-TOPIC-MAP; fix the layout and crate lines; move 023 to 02-guides and 033 to 05-operations.
7. `pidag-way-of-work.md:91`: "2; iteration 2 swaps to paid, then stop". `pii-mcp-bridge.md:23`: "`pii-mcp` binary".
8. technical-manual.tex: add the 2 methods and 2 crates; change "feature-gated" to "separate binary"; repoint 008 to 012.

**Duplication (canonical home):** gate steps go in `quality-gate.sh`, with one line in HANDOFF. Review staged-state goes in 042. Graph-service design goes in 037. The Chromecast conclusion goes in 038. Laya results go in 040. The crate list lives in `Cargo.toml`. The research file list goes in 000-TOPIC-MAP.

**NOT COVERED (docs)**

| item | reason |
|---|---|
| Bodies of 002 (270 KB), 026 (100 KB), 041-APPENDIX (1.6 MB) | headers, banners and links only, by design |
| `.tex` compilation and PDFs | read-only run; PDFs share the `.tex` date |
| Prose accuracy of 005, 007, 016, 020-022, 034 | status headers only |
| `/projects/pidag/docs/*`, `/projects/docs/01-overview/OPEN-REGISTER.md` rows | outside this repo |
| Host-infra claims in 040 (containers, ports) | not reproducible read-only |

## 10. Ranked candidate improvements (the next spec batch)

Each candidate is one bounded spec with an exit criterion a validator can check. Items 1-4 each have a test that fails today, and three of those tests already exist in the scratch copy.

1. **Fix the chunked decoder** (D1). In `crates/pii-net/src/service/dechunk.rs`, require `data_start + size + 2 <= raw.len()` before consuming. Consume the trailing CRLF only when both bytes are present. Reject a `\r` not followed by `\n`.
   - Exit: the split-point test (`/projects/_tmp/pii-review/mut-H6eG/src/crates/pii-net/src/service/dechunk.rs` `review_demo`) is copied into the repo and is red before and green after.
   - Also: `cargo test -p pii-net` green, and the gate green.
2. **Fix the bash render panic** (D2). Truncate bytes before `from_utf8_lossy` in `crates/pii-tools/src/bash.rs:113-117`, as line 106 does.
   - Exit: the 40,000-`é` test is red before and green after; `cargo test -p pii-tools` green.
3. **Make `reachability-check` real and gate it** (D3, Decision 5 step 1).
   - Correct both seed entries (`pii_model::resolve_with_transport(` in `crates/pii-server/src`; `sup.invoke(`).
   - Skip `#[cfg(test)]` bodies and `//` lines using the `lint_files` counter logic; delete the phantom `line_is_test` doc.
   - Replace the hardcoded path at `:171` with `CARGO_MANIFEST_DIR`.
   - Add the check to `quality-gate.sh`.
   - Exit: in a `git archive` scratch copy with `pii-server/src/lib.rs:203` changed to `provider_from_spec`, the check exits 1; on HEAD it exits 0; `grep -c reachability-check quality-gate.sh` is at least 1.
4. **Close the resume fail-open** (D4). In `engine/runs.rs`, propagate errors from `stored_run_scope_root`, the graph lookup and `get_run` instead of `.ok().flatten()`. A resume that cannot recover its confinement or budget fails loudly.
   - Exit: a test using the store fault seam asserts that resume returns `Err` and no run starts. It is red today.
5. **Make journal flush lossless and stop discarding its result** (D5). Drain only after `append_events` returns `Ok`, or restore `pending` on `Err`. Replace `let _ = self.flush()` at `journal.rs:165`, `run/engine.rs:332` and `sched/dispatch.rs:324` with an error event.
   - Exit: a fault-injected flush followed by a successful flush leaves contiguous seqs (red today).
   - Also: enable `clippy::let_underscore_must_use` for pii-graph, with zero violations.
6. **Tighten the spec-62/65/66/68 tests that guard nothing** (section 3).
   - spec-62: a call-site test with the validator stubbed out.
   - spec-65: hermetic per-test redb under `_tmp/<unique>`, plus a pii-store layer read-back test.
   - spec-66: one call-site message assertion in `session.rs`.
   - spec-68: an exit-code case of 128 or more, and delete the dead `& 0xff`.
   - Exit: each mutation named in section 3 turns at least one test red. The scripts are in `mut-H6eG`.
7. **Make the gate load-proof** (D9, D8). Replace the 10-second wall-clock deadline in `boss_watch.rs:188,205` with an event-driven wait or a 30-second ceiling.
   - Exit: `for i in $(seq 20); do cargo test -p pii-server --test boss_watch || break; done` passes while a parallel `cargo build` of a scratch tree runs.
8. **Route reactive-loop control events reliably** (D6). On `RecvError::Lagged` in `daemon/mod.rs:199`, rescan `runs_by_status(Suspended)` and re-arm; or give control events their own channel.
   - Exit: a test publishing 5,000 `AssistantDelta` events around a `RunSuspended` asserts the timer is armed.
9. **Remove xtask's hardcoded repo paths** (D7). Replace all 15 `/projects/pi-improved` literals with `workspace_root()`.
   - Exit: `grep -rn '"/projects/pi-improved' xtask | wc -l` is 0, and `cargo test -p xtask` passes from a `git archive` export at another path.
10. **Move heavy RPC handlers off the async workers** (A1). `spawn_blocking` for `memory.*`, `worker.reload`, `run.trace` and `session.prompt` in `engine/conn.rs:170`.
    - Exit: a current_thread heartbeat test per method asserts the heartbeat is never delayed by more than 50 ms.
11. **Documentation batch** (section 9, corrections 1-8). Docs only.
    - Exit: `grep -n "Nothing is committed" HANDOFF.md` sits under a "historical snapshot" banner; `grep -c "18 crates" AGENTS.md _docs/INDEX.md` is 0; `grep -n "035/036" _docs/06-research/030-OPUS-START-HERE.md` is empty; the 041 collision is resolved; `grep -c "run.resume" _docs/manuals/technical-manual.tex` is at least 1.
12. **Relocate `daemon/methods.rs` to `engine/methods.rs`** (Decision 2 follow-up).
    - Exit: `grep -rn 'crate::daemon' crates/pii-server/src/engine | grep -v '//'` is empty; gate green.
13. **Dependency hygiene** (F1, F2). Put `llama-cpp-2` behind a `llamacpp` feature in pii-model and drop unused dependencies.
    - Exit: `RUSTFLAGS=-Wunused-crate-dependencies cargo check --workspace` gives no warnings for non-test targets, and `cargo tree -p pii-tools | grep -c llama` is 0 without the feature.
14. **F-11 pass 3, `pii-store/src/lib.rs` (64 sites)**, with the existing constructor. This carries forward 032's item. Then the optional `pii-store/src/lib.rs` split, if wanted, which 032 ranked first among readability splits.
    - Exit: `grep -c "PiiError::Internal(" crates/pii-store/src/lib.rs` is 0; lint-files passes.
15. **Deferred, not to spec now:** the native code graph (Decisions 6 and 7), the modularity design (Decision 3), and Chromecast (Decision 4). Their triggers are in the rulings.

## 11. NOT VERIFIED

| item | why |
|---|---|
| Items marked I in section 7 (A1-A8, B1-B3, C1-C6, E1-E3, F2) | from a delegated read-only review; each needs the named test before it is acted on. A1, A2, D4, D5 and D6 were checked by me as far as their C-read labels say. |
| Whether D1's split occurs in production with Ollama | the split is proven to break decoding; how often real TCP reads land on that boundary was not measured |
| Whether a panic in D1 or D2 kills the daemon or only the request thread | not run against a live daemon (read-only run, nothing started) |
| `scripts/mvp-verify.sh` live suite | needs a running daemon and a model; out of scope for a read-only run |
| `cargo audit` / `cargo deny`, `cargo public-api`, `cargo machete` | not installed; installing tools would change the environment |
| Doc items marked I (about 79 of about 85 checks) | six re-checked by me, all held |
| graphify node and edge counts in 037 | the graph was not rebuilt; I only queried the existing one, which matches unchanged code |
| Benchmarks for the C-series allocation items | no bench was run; severities come from reading the code, not from a profile |

## 12. Single next most valuable change

**Fix the HTTP chunked decoder in `crates/pii-net/src/service/dechunk.rs`** (candidate 1).
- It is a demonstrated failure, not a hypothesis. A ready-made test splits a two-chunk body at every byte offset and fails at 5 of 16 points, one of them a panic in `http.rs`'s `drain`.
- It sits in the egress chokepoint that every streamed model turn uses (`ollama.rs:201`).
- The fix is about ten lines, and the proving test already exists.

The reachability-gate fix (candidate 3) is a close second. The project says its value is its verification seams, and that seam is currently proven blind.

## Scratch artifacts (outside the repo; move aside when done, do not delete blind)

| path | contents |
|---|---|
| `/projects/_tmp/pii-review/gate-20261006/` (172K) | all gate logs |
| `/projects/_tmp/pii-review/reach-zu2N/` (397M) | reachability mutation export and logs |
| `/projects/_tmp/pii-review/mut-H6eG/` (4.0G) | spec mutation export, copied target, the two `review_demo` tests, `run.sh`/`fresh.sh` |
| `/projects/_tmp/pii-review/opus-review-result-20261006.md` | this report |

Test runs also refreshed gitignored `crates/*/_tmp/` scratch inside the repo, which is normal test behaviour. No tracked file changed.