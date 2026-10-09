# BRIEF — re-review of the pii codebase after batches A–D (2026-10-09)

You are the SAME reviewer that produced
`_docs/06-research/043-opus-full-review-2026-10-06.md`. This is the RE-REVIEW pass of the
established review → implement → re-review pipeline: the ranked candidates from your §10 have
been implemented (batches A–D), and this pass judges the RESULT — the same way you judged the
2026-09-18 fixes in §3: by behaviour, with mutation tests where a test claims to guard something.

Read-only. You may (REVIEW_MODE=test) run `cargo`, the gate scripts and the xtask commands.
You may NOT edit, create or delete any file in the repo tree; scratch work goes under
`/projects/_tmp/` only. Do not commit.

## What changed since your review (all committed, tree clean)

- **Batch A** `a690e7e` D1 chunked-decoder split-safety (+ `f74de95` criterion form);
  `5c9a908` the **E0463 doctest fingerprint** root-cause — `[lib] doctest = false` added to all
  17 zero-doctest crates; `d7059cf` D2 bash render byte-cap; `a445126` D3 reachability-check
  rewritten (seeds corrected, `declare_dir` excluded, comments/cfg(test) skipped, hardcoded path
  gone, **wired into `quality-gate.sh`**); `9ab63a6` HANDOFF.
- **Batch B** specs 73/74: `PII_TEST_DEADLINE_SECS` (default 60) + self-describing failure
  messages in `boss_watch` (your candidate 7/D9); the four partly-guarded fixes from your §3 now
  each have a mutation-proven guard (62 both-off caller path, 65 hermetic + layer read-back,
  66 message order, 68 exit-255 case).
- **Batch C** specs 75–78: 75 docs-correction pass (your §9 items 1–8); 76 D7 (all 15
  `/projects/pi-improved` literals gone from xtask — paths from `CARGO_MANIFEST_DIR`) + F1
  (`llama-cpp-2` behind a `llamacpp` feature; default build does not link llama) + F2 unused-dep
  sweep; 77 D5 journal flush lossless (`pending` survives a failed append; the three discarded
  flush results now report through `report_flush_failure`) + `clippy::let_underscore_must_use`
  enabled at source for pii-graph; 78 D4 resume fails loud (scope/graph/budget recovery errors
  refuse the resume, no `u64::MAX` fallback, `register_run` only after recovery).
- **Batch D** specs 79–82: 79 D6 (a lagged control event now logs the gap and rescans
  `runs_by_status(Suspended)` to re-arm); 80 A1 (`memory.*`, `worker.reload`, `run.trace`,
  `session.prompt` run off the async worker via `spawn_blocking`; `SubscriptionRegistry` behind
  an `Arc`); 81 `daemon/methods.rs` → `engine/methods.rs` (pure `git mv`); 82 F-11 pass 3 (all
  64 hand-built store errors in `pii-store/src/lib.rs` → `PiiError::store_op`).
- **New test seams** (all test-only, all in the tree): `crates/pii-store/src/testing.rs`
  (fault injection: session writes, append_events, read_journal, get_run),
  `crates/pii-server/src/engine/conn_heartbeat_tests.rs` + `pub(crate) serve_connection_with`,
  `crates/pii-server/src/daemon/lag.rs`.

## What this pass must produce (be adversarial; your §3 method)

1. **Verify the landed fixes by behaviour.** For each batch above, decide: does the change fix
   the defect you reported, or does it move it? Use your own §3 method — mutate the guard and
   watch the test go red; delete the fix and watch something fail. Where a test exists, say
   whether it has teeth.
2. **Attack the E0463 fix specifically.** `doctest = false` on 17 crates makes a whole class of
   compile failure disappear. Is that a legitimate root-cause fix or a masking? What would now
   go unnoticed that previously surfaced? (Your §4 recorded "zero doctests of 17" long before —
   does this change anything for the future, or only silence the symptom?)
3. **Re-verify the claims the orchestrator accepted without you.** Two worth your attention:
   (a) spec 77's criterion originally passed `-W clippy::let_underscore_must_use` on the command
   line, which forced 19 blanket crate-level `allow`s outside pii-graph — those were REMOVED and
   replaced by a source-level `#![warn]` + 3 named allows; verify the lint is genuinely enforced
   in pii-graph today and NOT silently allowed elsewhere.
   (b) spec 82 was implemented by convert-then-test (RED proven by reverting one line, not
   observed first). Say whether the test actually pins the Display shape.
4. **The tests that claim teeth.** For 73/74/77/78/79/80, state which of them you can make go red
   by a mutation, and which are decoration. Two were already caught this way by the orchestrator
   (78's scope test passed for the wrong reason until it was seeded; 80's tests were flaky until
   the ticker accounted for the pending tick) — confirm those fixes hold and look for more of the
   same class.
5. **New defects.** Anything the batches introduced or exposed. Include the concurrency angle of
   80 (`run_handler` awaits before the next frame — is response ordering genuinely preserved for
   a client pipelining two requests, one heavy one light?) and the lag path of 79.
6. **Your §11 NOT VERIFIED list, updated**, and **a new ranked candidate list** with file:line
   and a pre-registered check per item (candidate → exit criterion a validator can run).
7. **The single next most valuable change**, in your words.

## Shape of the answer
- A markdown report, filed by the orchestrator to `_docs/06-research/044-opus-rereview-2026-10-09.md`.
- Start with a five-line verdict: which candidates actually landed, which claims held, which did not.
- Then per-item findings with `file:line`, and label every claim: `C-run` (you ran it),
  `C-read` (you read it), `I` (inferred).
- End with the ranked list and the NOT-VERIFIED section. No edits to the tree; no commits.
