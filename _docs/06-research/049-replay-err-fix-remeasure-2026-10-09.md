Experiment: fix(loop_budget=1) -> gate -> {pass: done | fail: retry -> fix}; test replay_err_divergence.
BEFORE (spec 93, unpatched replay):
  LIVE     = Failure { error: "loop_budget exceeded for node 'fix'" }
  REPLAYED = Failure { error: "no progress: node 'gate' never became ready" }
AFTER (spec 94): replay_matches_live_on_loop_budget_err passes:
  LIVE     = Failure { error: "loop_budget exceeded for node 'fix'" }
  REPLAYED = Failure { error: "loop_budget exceeded for node 'fix'" }
Reverted-replay RED assertion (verbatim):
  assertion `left == right` failed
    left: Failure { error: "loop_budget exceeded for node 'fix'" }
   right: Failure { error: "no progress: node 'gate' never became ready" }
