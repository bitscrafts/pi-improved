DIVERGENCE: with loop_budget=1, replay discards apply_completion's Err, so the resumed run ends "no progress: node 'gate' never became ready" while the live run ended "loop_budget exceeded for node 'fix'".

Test: replay_loses_activation_on_loop_budget_err (crates/pii-graph/tests/replay_err_divergence.rs)
Command: cargo test -p pii-graph --test replay_err_divergence -- --nocapture
Graph: fix(loop_budget=1) -> gate -> {pass: done | fail: retry -> fix}. Err path: `it == b` in apply_completion (loop budget), reachable because fix's only execution is the budget-th.

Observed (probe run, verbatim):
LIVE     = Failure { error: "loop_budget exceeded for node 'fix'" }
REPLAYED = Failure { error: "no progress: node 'gate' never became ready" }
Journal: 0 Started, 1 TickStarted[fix], 2 NodeStarted fix, 3 NodeFinished fix(ok), 4 Finished{Failure loop_budget exceeded}; resume appended 5 Resumed, 6 Finished{Failure no progress...}.
Parting step: journal seq 3 (NodeFinished fix). Live: dispatch::apply runs apply_failed_or_cancelled (fix->gate Dead), st.failed+=fix, failure=Some. Replay: edge fix->gate stays Pending, fix not in failed, failure None.

Control (budget=3): replay_matches_live_when_budget_err_is_followed_by_reentry — LIVE == REPLAYED ("loop_budget exceeded for node 'fix'"). Harmless there because iters is persisted before the Err and a later retry->fix re-entry makes fix ready again; the dispatch-time check (engine.rs iters >= budget) re-derives the failure.
Other Err path (re-entry into non-budgeted, non-cycle node) is unreachable via validation V3, not measured.

Fix spec would need: replay to handle the Err like dispatch::apply (apply_failed_or_cancelled + st.failed.insert + failure = Some(e)) — run_helpers.rs:125 and the allow comment.

Files touched: crates/pii-graph/tests/replay_err_divergence.rs (new). No production file changed.
