# Spec 287: a composed run through the REAL daemon process, from the CLI, with a stand-in agent

## Overview
**Project**: <repo>
**Phase**: production — composability, level 4 of 4 (free). Specs 284 and 285 drive `Engine` in a test process. The user's concern was the daemon itself: the **process** a queue worker talks to (`pii run start <graph>` over the control socket, in the devbox container, as the user `pii` runs). Between `Engine::start_run` and that is the wire, the CLI verbs, the daemon's own environment (the `PATH` the coder resolves agents from, the `env_pass` names it copies), the graph directory it loads from, and the binary actually installed (spec 279). None of that has run a composed graph.

This spec adds one **re-runnable check script** that starts the real tracked `spec` graph on the devbox daemon with a **stand-in agent** (a shell script, nothing spends), and reads the result back through the CLI the way an operator would.

## Verified Premises
- **CONFIRMED** (read 2026-10-26) the lane: `modules/devbox/daemon.sh` runs `pii` inside `pii-devbox` with `PII_RUNTIME_DIR=_tmp/devbox-test/run` and graphs in `_tmp/devbox-test/graphs`; `scripts/run-batchC-pii.sh` starts a run with `pii run start <name>` and reads `pii run get <rid>` and `pii run explain <rid>` (lines 146–170). `pii run trace` prints the `graph.call` lines (spec 270).
- **CONFIRMED** the coder resolves the agent binary directory from `PATH` (project rule; `agent_bin_dir` in `coder/paths.rs`); `env_pass = ["PATH", …]` copies the **daemon's** environment, so a stand-in is selected by the daemon's `PATH`, not by the run input.
- **CONFIRMED** the daemon refresh installs what it built and prints the digests (spec 279, `04e8203`), so the check can state which binary it exercised.
- **MEASURE FIRST** how to give the daemon a stand-in WITHOUT touching the production daemon's environment or a queue worker in flight: the check must start a **second, private daemon** (its own `PII_RUNTIME_DIR` under `_tmp/devbox-test/check-<ts>/run`, its own socket, its own graph dir, `PATH` = a stand-in bin dir first). Confirm that two daemons coexist (distinct runtime dir and socket); if the daemon refuses a second instance, record exactly why and fall back to **stopping nothing** — the check then refuses to run while a worker is in flight and uses a temporary graph copy, never the production one.
- **MEASURE FIRST** whether `pii run start spec --input '<json>'` accepts the input as the lane's own `run-batchC-pii.sh` passes it, and how a run's terminal status is read (`run get`'s `"status"`).
- **NOT CLAIMED** that this exercises the Anthropic or ollama transport: the stand-in is a shell script. Spec for the **real-agent** run is level 5 (the user's go-ahead; it spends).

## Requirements
### Functional
- **R1. `modules/devbox/composition-check.sh`** (committed, executable, `set -uo pipefail`, no absolute path outside the devbox mount, no credential read or printed). It: (1) refuses if any `run-batchC` or `claude`/`pi` worker process is running on the host **other than the check's own ancestors** (**AMENDED 2026-10-26**: the first version refused for every worker, and the worker implementing this spec is one, so it could never produce its verdict; a private daemon with its own runtime dir and socket touches nothing another worker owns, so only a worker that is NOT an ancestor of the check blocks it — read the ancestry from `/proc/<pid>/stat`'s ppid chain, and test the function against a recorded process table with a control where an unrelated worker is present and the check must still refuse); (2) creates a scratch run directory `_tmp/devbox-test/check-<ts>/` holding a bin dir with stand-in `pi` and `claude` scripts, a throwaway git repo with a spec (one criterion), and a copy of `graphs/spec.toml` + `graphs/docs.toml` (digest verified against the pin); (3) starts a private daemon in the container with that runtime dir and `PATH`; (4) runs `pii run start spec --input …`; (5) waits for a terminal status with a bounded timeout; (6) prints `run get`, `run trace`, `run explain`; (7) stops **only its own** daemon (by the pid it recorded, never by name), and moves the scratch dir under `_tmp/_trash/`.
- **R2. The check's verdict is a list of PASS/FAIL rows, each read from the daemon's own output**, ending `COMPOSITION-CHECK: <n> pass <m> fail`: (a) the run `Succeeded`; (b) `run trace` shows exactly one `graph.call … docs`; (c) the call digest equals `sha256sum graphs/docs.toml`; (d) two coder spawns with different sessions; (e) the budget: the stand-in records the `--max-budget-usd` it was given, and the second is smaller than the first by the first's reported cost; (f) the daemon binary's revision equals the repo's HEAD short sha, a `-dirty` suffix allowed and **printed** (a worker's tree is dirty by construction); any other difference prints `STALE-BINARY` and fails; (g) the private daemon's pid is gone afterwards and the production daemon still answers `health`.
- **R3. Controls inside the script**, each a second run on the same private daemon: a stand-in docs step that breaks the criterion → `Failed` with `reverify` the failing node; a run input missing `docs_brief` → refused before any agent spawns; a copy of `docs.toml` with one byte changed → refused with the pin mismatch.
- **R4. `modules/devbox/tests/composition-check-test.sh`** proves the script's own logic without a container: it sources the script's pure functions (the row parser, the verdict line) against recorded sample outputs, and a control feeds a sample with one row missing and asserts `fail >= 1`.
- **R5. First real run, recorded** in `_docs/05-operations/COMPOSITION-DAEMON-CHECK.md`: the date, the daemon revision and binary digest, and the verdict rows. A FAIL is a finding — recorded, with the journal excerpt, **not** fixed in this spec.

### Non-Functional
- No change under `crates/*/src` or `graphs/`. The production daemon's runtime dir, graph dir and environment are never modified. No `rm -rf`: scratch goes to `_tmp/_trash/`. Every file under the 450-line gate. No secret read, printed or written; the stand-ins hold none.

## Exit Criteria
- [ ] `test -x modules/devbox/composition-check.sh && bash -n modules/devbox/composition-check.sh`
- [ ] `bash modules/devbox/tests/composition-check-test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `grep -q 'PII_RUNTIME_DIR' modules/devbox/composition-check.sh`
- [ ] `grep -qE 'COMPOSITION-CHECK: [0-9]+ pass 0 fail' _docs/05-operations/COMPOSITION-DAEMON-CHECK.md`
- [ ] `git diff --quiet -- crates/pii-graph/src crates/pii-server/src graphs`
- [ ] `bash -n modules/devbox/check.sh && bash -n modules/devbox/daemon.sh`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `modules/devbox/composition-check.sh`
- `modules/devbox/tests/composition-check-test.sh`
- `_docs/05-operations/COMPOSITION-DAEMON-CHECK.md`

## Guardrails
- Do NOT stop, restart or reconfigure the production daemon. Do NOT run while a worker is in flight. Do NOT use `rm -rf`. Do NOT spend: the stand-ins are shell scripts. Do NOT commit.
- If a daemon-only defect shows (the graph passes in 284 but fails here), record it in the document and stop; do not edit source.

## Follow-ups
Level 5: the same graph with claude/haiku, capped at the spec's own `[limits]`, after the user's go-ahead; then the queue's worker moved onto `pii run start spec`.
