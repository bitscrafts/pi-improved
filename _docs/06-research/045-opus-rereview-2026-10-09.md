<!-- Filed 2026-10-09T11:18:27Z from an Opus plan-mode analysis run.
     Produced by: Claude Code + claude-opus-5-5, plan mode, READ-ONLY (no project writes).
     Brief: _docs/06-research/044-REREVIEW-BRIEF-2026-10-09.md
     Inventory the report was measured against: _docs/06-research/041-APPENDIX-codebase-inventory.txt
     Repo HEAD at analysis time: f26ee63 (tree: 0 changed paths)
     Run: session ef17e6d6-1741-42d6-87c6-16ffc775d231, stop_reason n/a, turns n/a, cost $n/a

     STATUS: NOT a spec and NOT a plan of record. Nothing in this document is authorised work.
     Every proposal needs its own pre-registered gate before implementation, and any idea previously
     closed by a measurement stays closed until a NEW measurement reopens it.

     ORCHESTRATOR NOTE (2026-10-09): filed as 045, not 044 — 044 is taken by the brief for this
     pass (044-REREVIEW-BRIEF-2026-10-09.md), so filing as 044 would have created the second
     collision the report itself flags in N9. The other shared number (041: BRIEF + APPENDIX) is a
     deliberate keep, recorded in 000-TOPIC-MAP T13 ("banners only, no renames").
     Anything marked "asserted-from-memory" is NOT verified — treat it as a lead, not a fact. -->

# 044: re-review of batches A to D (2026-10-09)

Provenance: the same reviewer as 043, in read-only test mode, on HEAD `4c52e79`. No repo file was edited and nothing was committed. Scratch work lives under `<scratch>/pii-review/`: `rereview-20261009/`, `rr-ab-CNHj/`, `rr-own-8sQT/` and `dtprobe-xgP6/`.

Two delegated runs did not finish: mutation of batches C and D, and the E0463 reproduction. Everything that depended on them is in section 4, not asserted here.

**Evidence labels:**

| label | meaning |
|---|---|
| **C-run** | I ran it |
| **C-run (orchestrator)** | the orchestrator reproduced it independently |
| **C-read** | I read the source lines |
| **C-delegated** | a delegated run executed the commands; logs are in its scratch directory |
| **I** | inferred |

> **Status banner (orchestrator, 2026-10-09, after the work-through).** Every ranked finding in this
> report has since been dispositioned, and the evidence is committed. Do NOT read this file as a live
> TODO list.
>
> | finding | disposition |
> |---|---|
> | N1 (non-hermetic tests; gate green on one machine) | **CLOSED**, and at the ROOT — specs 83, 84 (the `fresh-gate` tool), 85, 86 (sweep + retention/free-space guards), 96 (`tmp_scratch` helper + a FAILING `lint-scratch` gate step; 18 bare sites → 0). Evidence: `046-`…`051-` |
> | N2 (reachability foolable by `cfg(test)`, strings, comments) | **CLOSED** — spec 87 (whole-file lexer + `CfgTestGate`); mutation-proven |
> | N3 (lag recovery re-requests approvals; lost `RunFinished`) | **CLOSED** — spec 88 (`put_approval_if_absent` in one txn, no re-broadcast, never erases a decision; `RunFinished` recovered); mutation-proven |
> | N4 (llama provider compiled by no gate step) | **CLOSED** — spec 89 (gate step `clippy (llamacpp)`); teeth proven by an injected E0308 |
> | N5 (worktree `both-off` test weak) / N6 (spec-73 circular override test) | **UNCHANGED** — recorded as candidates; the reviewer's own reading stands |
> | N7 (no `shutdown_timeout`; 10 handlers still inline) | **CLOSED** — spec 95 (bound + log; all 11 handlers MEASURED, exactly one moved on the numbers) |
> | N8 (false `doctest = false` comments) | **CLOSED** — spec 90 |
> | candidate 6 (the mutation verification its delegated run never finished) | **DONE** — spec 91 (`046-`) filed 5 green mutants; spec 92 (`047-`) closed 4; the three redb-fault sites are the residual `pii R5b-residual` |
> | finding 6 (replay drops a state-changing `apply_completion` error) | **A REAL DEFECT, MEASURED AND FIXED** — spec 93 measured the divergence (`048-`), spec 94 fixed it and the recording test became the pinning test (`049-`); orchestrator reverse-mutated |
> | E0463 `doctest = false` narrative | **CORRECTED** — the flag cannot affect the gate (an explicit `--doc` ignores it); the 18 comments were rewritten by spec 90 |
>
> The gate now exists in two trees (repo + `fresh-gate` export); read
> `_docs/02-guides/quality-gate-and-fresh-tree.md` before trusting any "gate is green" claim.

## 1. Verdict

1. **Landed and proven by mutation:** D1 chunked decoder (spec 70), D2 bash render (71), and three of the four section-3 closures (65, 66, 68 in spec 74). The full gate is green on HEAD.
2. **Landed but only half-held:** D3 reachability (72) now catches a deleted caller. It is still satisfied by a reference inside a `#[cfg(test)]` module, a string literal, or a block comment. Its claim that "cfg(test) regions are excluded" is false.
3. **Did not hold:**
   - Spec 74's hermetic consolidate change exposed a fresh-clone gate failure in `pii-memory`.
   - Spec 79's lag recovery is not idempotent for approval suspensions.
   - Spec 74's worktree "both-off" test is guarded by git's ref-name rules, not by containment.
   - Spec 73's override test is circular, and it is `#[ignore]`d.
4. **The E0463 fix (`doctest = false`) changes nothing in `quality-gate.sh`.** An explicit `--doc` ignores the flag. It is a scoped workaround for plain `cargo test` runs, not a root-cause fix. It masks nothing in the gate, and its comments overclaim.
5. **Not verified this pass:** the mutation teeth of specs 77, 78, 79, 80 and 82, and the `let_underscore_must_use` enforcement question. Those runs did not complete.

## 2. Per-item findings

### Gate on HEAD

`bash quality-gate.sh` exited 0, every step PASS, including the new `reachability-check` step (C-run). Load was 2.7 at start and 7.3 at end.

| measure | value |
|---|---|
| `test result:` lines | 81 |
| passed | 585 |
| failed | 0 |
| ignored | 1 |

Log: `<scratch>/pii-review/rereview-20261009/quality-gate.log`. C-run.

The ignored count fell from 10 to 1. The single ignored test is `deadline_override_is_honoured`. The 9 real-model tests no longer compile in the default build, because they sit behind the new `llamacpp` feature (C-run for the count, C-read for the feature gate). See N4.

### Batch A

**Spec 70, chunked decoder (D1). GUARDED, with one gap.**
- Reverting the whole file, dropping or shifting the `+ 2`, removing the trailer wait, and letting `find_crlf` accept a lone `\r` each turned `every_split_point_decodes` red, among other tests (C-delegated).
- HEAD passed a wider single, double and byte-by-byte split test. It covered a 3-chunk body with an extension (`5;ext=1`), a trailer, a non-ASCII extension, and two trailers. There were no errors, panics or leftover bytes (C-delegated).
- **Gap:** removing the R1.1 partial-line wait stays green under the shipped tests, 4 of 4 runs. Only the non-ASCII-extension body kills that mutant (C-delegated).

**Spec 71, bash render (D2). GUARDED.**
- Reverting to `String::truncate` after decoding turns `renders_multibyte_output_at_the_cap` red with a panic at `bash.rs:117`. Moving the cap by one byte turns three tests red (C-delegated).
- **Gap:** changing `>` to `>=` at the cap survives, because no test has output of exactly `MAX_OUTPUT` or `MAX_OUTPUT + 1` bytes (C-delegated).

**Spec 72, reachability (D3). PARTIAL.**

What holds:
- With the real `resolve_with_transport` caller removed, the check exits 1 and names it (C-run, my export `rr-own-8sQT`).
- `quality-gate.sh:79-89` turns a failure into gate exit 1 (C-delegated).

What does not hold:
- With the caller removed and a `#[cfg(test)] mod review_probe { fn f() { let _ = pii_model::resolve_with_transport("x", todo!()); } }` stand-in added, the check exits 0. That is (C-run) by me and (C-run by orchestrator, reproduced).
- These stand-ins also fool both seed entries, with exit 0 each (C-delegated):
  - a string literal containing the call;
  - a single-line `/* */` block comment;
  - a multi-line `/* */` block comment;
  - a multi-line raw string;
  - `#[cfg(test)] fn`;
  - `#[cfg(test)]` followed by `#[allow]` and then `mod`.
- `//`, `///` and `//!` lines are correctly ignored (C-delegated).

Root cause: `xtask/src/reachability.rs:178-186`. `effective_lines` calls the block-aware `count_effective_lines(t)` on one trimmed line at a time, so the `#[cfg(test)]` state can never carry across lines (C-read).

Test teeth:
- `cfg_test_body_is_not_a_caller` is decoration. Its fixture sits under `declare_dir`, so it passes for the wrong reason, and it goes red only when the `declare_dir` skip is disabled (C-delegated).
- Disabling the `is_test_path` skip is not caught by any test (C-delegated).
- Turning `effective_lines` into a no-op is caught only by integration tests (C-delegated).

**E0463 and `doctest = false` (commits `5c9a908`, `a690e7e`): a workaround, not a root-cause fix, and not masking in the gate.**
- In a scratch crate with `[lib] doctest = false` and a deliberately wrong doctest, plain `cargo test` reports "running 0 tests" and exits 0. `cargo test --doc` runs the doctest, fails, and exits 101 (C-run, `dtprobe-xgP6`).
- So the flag is ignored by an explicit `--doc`.
- `quality-gate.sh` never runs a plain `cargo test`. Its test step selects `--lib --bins --tests`, and its doc step uses `--doc` (C-read). Today's gate log shows `Doc-tests` for all 17 library crates, each "running 0 tests" (C-run).

What follows:
- **(a)** The change cannot have altered the gate. The `Cargo.toml` comments claim "the shipped quality gate can never pass", and the commit message says "63/63 green after". Neither can be causally true of `quality-gate.sh` (C-read and C-run).
- **(b)** Nothing is masked in the gate. A future doctest in any of the 17 crates would still be compiled and run by the gate's `--doc` step.
- **(c)** The only effect is on plain `cargo test` runs, such as spec exit criteria, `cargo test -p X` and `cargo test -j2`. Those no longer run a doc phase that had zero tests. That is a legitimate way to avoid a cargo-level fingerprint problem in shared target directories. The underlying cargo behaviour is untouched, so it is a workaround (I).
- Whether E0463 still reproduces with the flag removed: NOT VERIFIED (section 4).

### Batch B

**Spec 73, boss_watch deadline (D9). The behaviour holds; the guard test is decoration.**
- `PII_TEST_DEADLINE_SECS=1` fails with a self-describing message: `timed out waiting for journal growth: condition journal len >= 6 (deadline 1s, last journal len 5)`. The default is 60 s (C-delegated).
- If `test_deadline_secs()` ignores the environment variable, `deadline_override_is_honoured` still passes. It computes its expectation through the same helper, so it is circular. It is also `#[ignore]`d (C-delegated).
- Fixed short wall-clock deadlines remain in other pii-server tests (C-delegated):
  - `tests/reattach.rs:47`
  - `tests/slice_f_promotion.rs:68`
  - `tests/model_routing.rs:149`
  - `tests/spawn_node.rs:96`, `:126`, `:218`
  - `tests/latency.rs:97` (`< 50` ms)
  - `src/node_kinds/agent/tests.rs:442` (`< 2` s)

**Spec 74, the four closures:**

| closure | verdict | evidence |
|---|---|---|
| 62 worktree, both checks off | **weak** | `worktree_join_refuses_path_traversal_payload` goes red only because git rejects the ref name `heal/../../../etc` (exit 255). The test pins `Invalid` but never checks containment itself (C-delegated). With the validator on and the call-site check off it stays green, as the test admits (C-delegated). |
| 65 layer persistence | **guarded and hermetic** | dropping the `MEM_LAYER` insert is red on two consecutive runs in `pii-store` and `pii-memory`; consolidate now uses `tempdir_in` (C-delegated) |
| 66 store_op display order | **guarded** | (C-delegated) |
| 68 exit code 255 | **guarded** | the remaining survivors (`& 0xfff`, `clamp(0,255)`) are equivalent mutants (C-delegated) |

### Batch C

**Spec 75, documentation.** My 043 section-9 items 1, 2, 3, 7 and 8 landed, along with the topic-map fix (C-run greps):
- HANDOFF has the historical banner at `:190`.
- AGENTS and INDEX say 19 crates.
- 030 has no `035/036` references.
- `pidag-way-of-work.md:91` says 2 iterations.
- `pii-mcp-bridge.md:23` names the `pii-mcp` binary.
- The technical manual lists `run.resume`, `run.trace`, `pii-uds` and `pii-http-fetch`.
- The "Nothing has been deleted" line is gone from the topic map.

Not fixed: 041 is still used twice (APPENDIX and BRIEF). Filing this report as `044-opus-rereview` would make 044 a second collision, next to `044-REREVIEW-BRIEF-2026-10-09.md` (C-run `ls`).

**Spec 76, llama feature (F1).**
- `llama-cpp-2` is optional behind `llamacpp = ["dep:llama-cpp-2"]` (`crates/pii-model/Cargo.toml:19-21`), with `cfg(feature)` gates at `pii-model/src/lib.rs:214-378` (C-read).
- `llamacpp_spec_errors_when_feature_off` passes in the gate (C-run).
- `quality-gate.sh` never enables the feature (C-read), so the provider code is no longer compiled, linted or tested by the gate. See N4.
- The xtask path cleanup (D7) is partly evidenced: in a scratch export, `reachability-check` and the `pii-memory` tests resolved paths inside the export, not inside `<repo>` (C-run). The full grep was not re-run (section 4).

**Spec 77, journal flush (D5) and the `let_underscore_must_use` question.** NOT VERIFIED (section 4).

**Spec 78, resume fails loud (D4).** NOT VERIFIED (section 4).

### Batch D

**Spec 79, lag recovery (D6). The code is read; the fix does not fully hold.**
- `daemon/lag.rs` `recover_suspended` calls `on_suspended` for every `Suspended` run. Its doc claims it is "Idempotent — the wheel is keyed by (fire time, run)" (C-read).
- That is true for timers, because the fire time is parsed from the key (`daemon/mod.rs:343-346`, C-read).
- It is false for approvals:
  - `on_suspended`'s approval arm (`mod.rs:348-350`) calls `approvals::request`.
  - `approvals::request` (`daemon/approvals.rs:48-75`) unconditionally `put_approval`s a row with `decision: None` and a fresh `requested_at_ms`, then re-broadcasts `ApprovalRequested` (C-read).
  - The result is N1.
- Lost `RunFinished` events are not recovered at all. `on_finished` (`mod.rs:358-385`) re-arms cron and sends channel replies, and the rescan never touches it (C-read). 043 D6 named both event kinds.
- The test's teeth are NOT VERIFIED.

**Spec 80, heavy methods off the async worker (A1). The code is read; the test teeth are not verified.**

Ordering is preserved:
- `serve_connection_with` awaits `run_handler(...)` before reading the next frame (`engine/conn.rs`, diff of `b65572a`).
- Two pipelined requests, heavy then light, are answered in request order. The light one waits behind the heavy one (head-of-line), exactly as before. There is no regression (C-read).
- A panic inside `spawn_blocking` now becomes an `Internal` error response instead of killing the connection task, which is an improvement (C-read).

Cancellation does not reach the work:
- A client that disconnects mid-request does not stop the blocking task. It runs to completion, and only then does `write_resp` fail and break (C-read).
- The daemon runtime is entered with `rt.block_on(...)` at `pii-cli/src/cmd_daemon.rs:158` and dropped without `shutdown_timeout` (C-read).
- So shutdown now waits for any in-flight `memory.consolidate` or `worker.reload` (I).

Scope is as specified. These still run inline with store or file I/O (C-read):
- `dispatch.rs:54` (`SessionCreate`)
- `:94` (`RunCancel`)
- `:111-113` (`RunStart`, `RunGet`, `RunList`)
- `:133` (`ApproveResolve`)
- `:141` (`SysReload`)
- `:142` (`RunResume`)
- `:161` (`SessionClose`)
- `:174` (`GraphApply`)
- `EventSubscribe` replay (I)

**Spec 81, method relocation. Landed.** `engine/dispatch.rs:133-142` now calls `crate::engine::methods::*` (C-read). The full `crate::daemon` grep over `engine/` was not re-run.

**Spec 82, F-11 pass 3.** NOT VERIFIED (section 4).

### The two orchestrator-accepted claims

- **(a) Spec 77 lint enforcement:** NOT VERIFIED. The run that would answer it did not complete.
- **(b) Spec 82 Display pinning:** NOT VERIFIED, for the same reason.

## 3. New defects

| # | sev | location | defect | evidence |
|---|---|---|---|---|
| N1 | **HIGH (gate)** | `crates/pii-memory/tests/lanes.rs:123`, `:158`, `:188` | Three tests open hardcoded `concat!(CARGO_MANIFEST_DIR, "/_tmp/phase3/...")` paths and never call `tmp_root()` (`:23-25`, which creates `phase3`). On a fresh tree, `degrades_honestly_without_embedder`, `graph_lane_recalls_doc_via_shared_concept` and `memory_hit_carries_doc_text` fail with "No such file or directory". **Any clean clone, CI run or disposable worktree, which is the coding harness's own model, gets a red gate.** | C-run (me, `rr-own-8sQT`), C-run (orchestrator, reproduced), C-delegated |
| N2 | **HIGH (gate)** | `xtask/src/reachability.rs:178-186` | Reachability is satisfied by test-only, string-literal and block-comment references, because the cfg(test) state is evaluated one line at a time. | C-run (me and orchestrator), C-delegated, C-read |
| N3 | MEDIUM | `daemon/lag.rs` `recover_suspended` → `daemon/mod.rs:348-350` → `daemon/approvals.rs:48-75` | On every lag, recovery overwrites each pending approval row (decision reset to `None`, timestamp reset) and re-broadcasts `ApprovalRequested`, so humans get duplicate notifications. In the window between `approve.resolve` recording a decision and the run leaving `Suspended`, the decision is erased (I). Lost `RunFinished` events (cron chain, channel reply) are still never recovered. | C-read; the race window is I |
| N4 | MEDIUM | `quality-gate.sh` (no `--features llamacpp` anywhere); `crates/pii-model/src/lib.rs:214-378`; `llamacpp.rs` | The in-process llama provider and its 9 real-model tests are compiled by no gate step. They can stop compiling unnoticed, including the open A6 cancellation issue. | C-read; ignored count 10 to 1 is C-run |
| N5 | LOW-MED | `crates/pii-server/tests/worktree_join_containment.rs` | The spec-62 both-off test is red only because git rejects the ref name. It never asserts containment, so the call-site containment check has no direct guard. | C-delegated |
| N6 | LOW-MED | `crates/pii-server/tests/boss_watch.rs` (`deadline_override_is_honoured`) | The test is circular (its expectation comes from the helper under test) and `#[ignore]`d. | C-delegated |
| N7 | LOW | `pii-cli/src/cmd_daemon.rs:158` | No `shutdown_timeout`. Spec 80 makes in-flight blocking work the norm for heavy methods, so exit can wait on it. | C-read; impact I |
| N8 | LOW | 18 `Cargo.toml` `[lib] doctest = false` comments | They assert a gate effect the flag cannot have. A reader will believe the gate's doc step was disabled; it was not. | C-run probe, C-read |
| N9 | LOW | `_docs/06-research/` | Number collisions: two files are 041, and filing this report as 044 makes two 044s. | C-run |

**Root cause of N1.** The root cause is `lanes.rs` itself: three tests depend on a directory they never create. That latent order dependency predates spec 74. Before `5712f1c`, `tests/consolidate.rs` ran `create_dir_all(root.join("phase3"))`, and its binary runs before `lanes` in cargo's order, which hid the bug.

Spec 74 replaced that line with `create_dir_all(&root)` plus `tempdir_in`, as shown in the `5712f1c` diff. It is the change that removed the accidental crutch, not the defect (C-read).

It also follows (I) that `cargo test -p pii-memory --test lanes` alone has failed on a fresh tree since before spec 74. The other `lanes` tests that do call `tmp_root()` run on parallel threads, so even the old crutch was order-dependent.

## 4. NOT VERIFIED

| item | why |
|---|---|
| Spec 77: does the flush test go red when `pending` is drained before the append? Is `let_underscore_must_use` enforced in pii-graph, with no crate-level allow elsewhere? Are the 3 named allows justified? | the batch C/D mutation run stopped before reporting |
| Spec 78: does each of the scope, graph, budget and register-order mutations turn a test red? Does the orchestrator's seeded scope test still fail if the fault seam is a no-op? | same |
| Spec 79: does `lagged_control_event_recovers_suspended_timer` go red with the rescan removed? Does it create a genuine broadcast lag? | same |
| Spec 80: does each heartbeat test go red when `is_heavy` is false or a method is dropped? Is the light-method control real? Is the flakiness fix holding? | same |
| Spec 82: `PiiError::Internal(` count in `pii-store/src/lib.rs`; does `open_failure_renders_structured_store_op` pin the full string or a substring? Are call-site argument swaps caught? | same |
| Whether E0463 reproduces with `doctest = false` removed, under the check-then-`test -j2` sequence; duplicate rlib evidence | the reproduction run stopped |
| Spec 76: full `grep -rn '<repo>' xtask crates` re-run; `cargo tree` llama counts per crate; `--features llamacpp` still compiles | not re-run this pass (partial C-run evidence in section 2) |
| Spec 81: `grep 'crate::daemon' crates/pii-server/src/engine` empty | not re-run |
| N3 race window in practice | needs a timed test |
| 043 section-11 carry-overs still open: A2 to A8, B1 to B3, C1 to C6, E1 to E3, F2 and F3; live-daemon behaviour of panics; `cargo audit`/`machete`/`public-api` | not in this batch scope; tools not installed |

## 5. Ranked candidates (pre-registered checks)

1. **Make `lanes.rs` hermetic (N1).** Route the three tests at `crates/pii-memory/tests/lanes.rs:123`, `:158`, `:188` through `tempfile::Builder::tempdir_in(tmp_root())`.
   - Check: `S=$(mktemp -d <scratch>/x-XXXX); git archive HEAD | tar -x -C $S; cd $S && CARGO_TARGET_DIR=$S/t cargo test -p pii-memory` exits 0 with no `_tmp` present.
   - Check: `grep -c '"/_tmp/phase3' crates/pii-memory/tests/lanes.rs` is 0.
2. **Add a fresh-tree gate run** so the N1 class cannot recur. Add `xtask fresh-gate` or a CI job that runs `quality-gate.sh` inside a `git archive` export.
   - Check: on today's HEAD it exits non-zero, naming the three `lanes` tests.
   - Check: after candidate 1 it exits 0.
3. **Make reachability stateful (N2).** In `xtask/src/reachability.rs:178-186`, run the cfg(test) state machine over the whole file, and strip string literals, raw strings and block comments before matching. Move the `cfg_test_body_is_not_a_caller` fixture out of `declare_dir`.
   - Check: each of the 8 stand-in variants in `rr-ab-CNHj` (cfg(test) mod, cfg(test) fn, attribute stack, string literal, raw string, single-line and multi-line block comment) makes the check exit 1 with the real caller muted.
   - Check: HEAD exits 0.
   - Check: disabling the cfg(test) skip turns `cfg_test_body_is_not_a_caller` red.
4. **Make lag recovery idempotent for approvals, and recover lost `RunFinished` (N3).** In `daemon/mod.rs:348-350` or `approvals.rs:48`, skip `put_approval` and the broadcast when a row already exists. On `Lagged`, also re-arm cron entries that have no live run.
   - Check: a test decides an approval, keeps the run `Suspended`, calls `recover_suspended`, and asserts the decision is unchanged and zero `ApprovalRequested` events were sent. It is red today.
   - Check: a test drops a cron run's `RunFinished` under lag and asserts the next firing starts.
5. **Compile the llama provider in the gate (N4).** Add `cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings` (or `cargo check`) to `quality-gate.sh`.
   - Check: `grep -c 'features llamacpp' quality-gate.sh` is at least 1.
   - Check: in a scratch export, a type error injected in `crates/pii-model/src/llamacpp.rs` makes the gate exit non-zero.
6. **Finish the batch C/D verification** listed in section 4, using the spec-by-spec mutation list already written for that run.
   - Check: each of 77a, 77b, 78a-e, 79a-b, 80a-c and 82b-c has a recorded baseline-green, mutant-red result. Any green mutant becomes its own candidate.
7. **Give the worktree containment check a direct test (N5).** Test the call-site check with the validator stubbed, using a payload git accepts, or assert that no directory was created and the error came from the containment check.
   - Check: with the call-site check disabled and the validator on, the test is red.
8. **Make the spec-73 override test real (N6).** Set `PII_TEST_DEADLINE_SECS=1` against a literal bound (fails in under 5 s), and drop `#[ignore]`.
   - Check: the "ignore env var" mutant turns it red.
9. **Close the spec-70 and spec-71 boundary gaps.**
   - Check: a partial-size-line test with a non-ASCII extension turns the "R1.1 wait removed" mutant red.
   - Check: render tests at exactly `MAX_OUTPUT` and `MAX_OUTPUT + 1` turn the `>` to `>=` mutant red.
10. **Put the remaining fixed test deadlines on `PII_TEST_DEADLINE_SECS`.** Files: `reattach.rs:47`, `slice_f_promotion.rs:68`, `model_routing.rs:149`, `spawn_node.rs:96,126,218`, `latency.rs:97`, `agent/tests.rs:442`.
    - Check: `cargo test -p pii-server` passes 3 times in a row while a parallel scratch `cargo build` runs.
11. **Take the remaining blocking inline methods off the worker and bound shutdown (N7).** Methods at `engine/dispatch.rs:54`, `:94`, `:111-113`, `:133`, `:141`, `:142`, `:161`, `:174`; add `rt.shutdown_timeout` at `pii-cli/src/cmd_daemon.rs:158`.
    - Check: one heartbeat test per method.
    - Check: a SIGTERM test with a 5-second blocking handler in flight exits within 2.5 s.
12. **Documentation only (N8, N9).** Correct the 18 `doctest = false` comments to say the flag affects plain `cargo test` only. Fix the 041 collision and file this report as 045.
    - Check: `grep -rl 'can never pass' crates xtask` is empty.
    - Check: `ls _docs/06-research | cut -c1-3 | sort | uniq -d` is empty.
13. **Carry forward the open 043 items not yet specced:** A2 client hang, A3 childproc drain, A4 unbounded connection channel, A5 accept loop, A6 llama cancellation, A8 worker pid check, B1 discarded results outside pii-graph, and C1-C5 hot-path clones. The checks are as pre-registered in 043 section 7.

## 6. Single next most valuable change

**Make the three `lanes.rs` tests hermetic, then add a fresh-tree gate run (candidates 1 and 2 together).**

Today the gate is green only on the one machine that already has `crates/pii-memory/_tmp/phase3`. Every clean clone, every CI checkout and every disposable worktree fails it. Disposable worktrees are exactly how the approved coding harness will run.

A gate that depends on leftover state is the failure mode this project's gates exist to prevent. The fix is a few lines, and the fresh-tree run stops the whole class from coming back.