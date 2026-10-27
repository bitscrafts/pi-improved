# Composition check through the real daemon (spec 287): runs

## Run 2 (the verdict)

**Date:** 2026-10-26. **HEAD:** `52c61363bf4b`. **Daemon:** `_tmp/devbox-test/bin/pii`, revision `52c61363bf4b`, installed by `daemon.sh refresh` (`installed d2f357b3e4ed`, daemon binary sha `d2f357b3e4ed`, the two digests equal). No worker in flight; the loop was paused. Stand-in agents only: nothing spent, and nothing here exercises the Anthropic or ollama transport.

```
CONTROL break PASS: docs step broke the criterion: failed, reverify named
CONTROL missing-docs_brief PASS: refused before any spawn (none)
CONTROL pin PASS: docs.toml with one byte changed refused (none)
PASS a run Succeeded (status: succeeded)
PASS b run trace shows one call docs (saw 1)
PASS c the docs call digest is sha256sum graphs/docs.toml (b7f8eacdb79a)
PASS d two coder spawns with different sessions
PASS e the second budget (0.75) is the first (1) less the reported cost 0.25
PASS f daemon revision 52c61363bf4b is HEAD 52c61363bf4b
PASS g private daemon gone and production daemon answers health
COMPOSITION-CHECK: 7 pass 0 fail
```

The private daemon was started and stopped by its own pid; the production daemon answered health before and after.

## Run 1 (history)

The first real run, at `8056c3f4439d` against the then-production daemon `7ca198a1574d`, printed `COMPOSITION-CHECK: 6 pass 1 fail`: row (f) failed (the daemon was stale) and the missing-`docs_brief` control failed (the run was refused only after the host's `code` coder had spawned, which spec 288 closed). The worker implementing the check had also found that its own guard refused for every worker, itself included, which spec 287's amendment fixed.

## What this proves, and what it does not

- Proven: the tracked `spec` graph (`code`, `verify`, the `docs` call, `reverify`, `done`) runs through the real daemon process from `pii run start`; one `graph.call` carrying the digest of `graphs/docs.toml`; two coder sessions; the budget shared through the call (the second cap is the first less the first's reported cost); a docs step that breaks the criterion is caught by `reverify`; a missing `docs_brief` is refused before any agent spawns; a changed pin is refused.
- Not proven: any real agent, a run longer than the stand-ins' milliseconds (the wall limit of spec 290 is tested by derivation, not by a slow run), a daemon restart mid-call against the real process (spec 289 is tested through the engine), and concurrency against the production queue.
