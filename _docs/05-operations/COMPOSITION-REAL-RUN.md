# Composition: a real run through the daemon (spec composability, level 5)

**Date:** 2026-10-27. **HEAD:** `3cc4ffe5a397`. **Daemon:** the production devbox daemon, revision `3cc4ffe5a397` (== HEAD), installed by
`daemon.sh refresh` (`installed 20adef32d275`, daemon binary sha `20adef32d275`). **Agent:** the real `claude` on `claude-haiku-5-5`, provider
`anthropic`, through the real transport. **Cap:** USD 1.00 (graph `[limits]` and the input's `max_budget_usd`). **Repo:** a throwaway git repo
under `_tmp/devbox-test/`; the live project was never the workdir. Run with `scripts/archive/composed-real-run.sh` (and `--control`).

## The run (r000000000322)

```
FACTS   : spawns=2 exits=2 calls=1 distinct_sessions=2 rc=[0 0 ] turns=[3 3 ] cost=[0.002670 0.002264 ] total=$0.004934 quota_exhausted=[no ]
PASS the run succeeded ("status":"succeeded")
PASS two coders spawned and exited
PASS two distinct agent sessions
PASS run trace shows one call docs carrying the digest of docs.toml (b7f8eacdb79a)
PASS hello.txt holds PONG
PASS the docs coder wrote NOTES.md
PASS nothing was committed by a worker
PASS total cost $0.004934 is under the $1.00 cap
PASS no quota refusal
COMPOSED-REAL: 9 pass 0 fail   (evidence: _tmp/claude-batchC/composed-real-1793068356.log, _tmp/devbox-test/real-1793068356/journal-lines.txt)
```

## The control (r000000000325): the same graph, a docs coder told to break the criterion

The docs brief carries a requirement change: overwrite `hello.txt` with `DOCS`. The host criterion is `grep -qx PONG hello.txt`, so `reverify` must refuse.

```
FACTS   : spawns=2 exits=2 calls=1 distinct_sessions=2 rc=[0 0 ] turns=[4 4 ] cost=[0.002830 0.002845 ] total=$0.005675 quota_exhausted=[no ]
PASS CONTROL: the run FAILED because the real docs coder broke the criterion ("status":"failed")
PASS CONTROL: hello.txt holds DOCS, the sabotage really happened
PASS CONTROL: both coders ran and the call is in the trace
PASS CONTROL: the trace reaches the unverified refusal
PASS CONTROL: total cost $0.005675 is under the $1.00 cap
COMPOSED-REAL: 5 pass 0 fail   (evidence: _tmp/claude-batchC/composed-real-1793068511.log, _tmp/devbox-test/real-1793068511/journal-lines.txt)
```

The control ran twice and failed the same way twice (r000000000324, r000000000325).

## Findings, including the ones against me

- **A blunt sabotage was refused by the real agent.** The first control brief said to delete `hello.txt`. The real docs coder read the spec, answered that the
  instructions conflict, and changed nothing (run succeeded, `hello.txt` intact). A stand-in obeys; a real model does not. The control was rewritten as a
  requirement change to the file's content, which the model obeyed. Had it refused that too, the finding would have been "not achievable with this model".
- **A real model complies with a plausible instruction that contradicts the spec.** That is exactly why the docs step is followed by `reverify`: the host's
  criteria, not the docs coder's judgement, decide. The control shows it working with the real transport.
- **Two defects in my own script were found by running it, not by reading it:** the first verdict row counted a word in the daemon's log, which carries coder
  events only (the call is in `run trace`, so it read `graph.call=0` on a run whose trace showed the call; fixed and proven on the saved trace, with the call line
  removed and one digest character changed as controls); and a background shell has no `uv` on its PATH (fixed, and the script is now run under `env -i`).
- **Cost:** USD 0.004934 for the passing run (3 and 3 turns), USD 0.005675 for the control. The cap was never close.

## What this proves, and what it does not

- Proven: the tracked `spec` graph (`code`, `verify`, the pinned `docs` call, `reverify`, `done`) runs through the production daemon with the real claude on haiku.
  Two real sessions, one call carrying the pinned digest, the budget charged from real reported cost, nothing committed by a worker, and a docs step that breaks the
  criterion is caught by `reverify` with the real transport.
- Not proven: a run near the wall limit (these finish in 15 to 25 seconds, so spec 290's limit is proven by derivation and not by a slow run); a daemon restart or
  cancel mid-call with a real agent; quota exhaustion mid-call; concurrent real runs; a spec that needs a real implementation (this one writes one file);
  pi or ollama through the composed graph; the queue's worker (still a one-node graph, not `pii run start spec`).
