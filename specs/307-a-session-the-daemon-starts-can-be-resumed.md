# Spec 307: a session the daemon starts can be resumed, and the lane only says "found" when the conversation file exists

## Overview
**Project**: /projects/pi-improved
**Phase**: production — composability, resume (found 2026-10-28 02:0xZ on A168's resume, run r000000000522: `exited 1 (agent failed: No conversation found with session ID: 87830677-...)`)
The devbox daemon runs under landlock `workspace`: writes are confined to the workspace, the store, the runtime dir and `$TMPDIR` (`crates/pii-tools/src/sandbox.rs`). Claude writes its conversation file under `$HOME/.claude/projects/<cwd>/<sid>.jsonl` (`/root/.claude/projects`, a volume), which that profile refuses, so a coder the daemon starts leaves NO conversation file and `claude --resume <sid>` can never find it.
MEASURED 2026-10-28: the same `claude --session-id U` then `--resume U`, run directly in the container (no sandbox), writes `/root/.claude/projects/-projects-pi-improved--tmp-devbox-test-probe-session-A/U.jsonl` and resumes ("PONG"). Run by the daemon, sessions `87830677-...` and `48e5b69d-...` left only an empty `tmp/claude-0/<project>/<sid>/tasks` directory and the container's sessions volume holds ONE transcript in total.
The lane's check (`run-batchC-pii.sh`, `find $C/tmp -name "<sid>*"`) matches that empty task directory, so it printed `RESUMING ... (found in the devbox)` and spent a run that could only fail. Every resume of a queue row since the composed lane began has been dead for this reason; spec 303's stub tests could not see it.

## Requirements
### Functional
- R1: `modules/devbox/daemon.sh` starts the daemon with `CLAUDE_CONFIG_DIR=$C/claude`, a directory inside the workspace, created by the script, holding a symlink `.credentials.json -> /run/claude-login/.credentials.json` (the same login file the image already links). The exact mechanism is the worker's to choose, but the result must be measured, not assumed: a coder started by the daemon leaves `<sid>.jsonl` under a path the sandbox allows, and a second run with `sid = <that session>` resumes it.
- R2b (measured 2026-10-28): the control row must keep claude's LOGIN valid and drop only the writable session store. Unsetting `CLAUDE_CONFIG_DIR` where the login lives makes claude say "Not logged in", which proves nothing about sessions. The control starts the private daemon WITHOUT the writable `CLAUDE_CONFIG_DIR` but with the same login mount the production daemon has, and its resume step must fail with `No conversation found`. If it fails with any other text, the control is invalid: the script prints `CONTROL INVALID: <text>` and exits non-zero, and the worker reports it instead of changing the expected text.
- R2: `scripts/checks/resume-through-daemon.sh` proves R1 with a REAL claude (haiku, two one-line prompts, a few cents) through a PRIVATE daemon started the way `daemon.sh` starts it (its own runtime dir, store and graph dir; the production daemon is never touched, never restarted). It runs a one-node coder graph that asks "Reply with exactly: PONG", reads the session id from the journal's `coder.spawned`, then starts a second run with that `sid` and a prompt that can only be answered from the first ("What single word did I ask you to reply with?") and asserts the answer contains PONG. Output ends `RESUME-THROUGH-DAEMON: OK` or `... FAIL <why>`. CONTROL, in the same script: with `CLAUDE_CONFIG_DIR` unset, the same two steps FAIL with `No conversation found`, so the check can fail. Both outcomes are printed as rows.
- R3: `scripts/run-batchC-pii.sh` decides "found" by the conversation file itself: `find <config dir> -name "<sid>.jsonl"`, where the config dir is the one R1 chose. A directory named for the session is not a hit. When it is not found the existing `RESUME: session <sid> not found in the devbox, starting fresh` line is printed (stdout and worker log). `scripts/autonomy/tests/lane-resume.test.sh` gains a case with an EMPTY `tasks` directory named for the session that must print `not found`, and a case with a real `<sid>.jsonl` that must print `RESUMING`.

### Non-Functional
- Shell only. Never restart the production daemon from a criterion. No credential value is printed or written to a file in the repository; the symlink target is the image's existing mount.

## Verified Premises
- **CONFIRMED** (above) direct `--session-id` then `--resume` works; the daemon's spawns leave only an empty `tasks` directory.
- **CONFIRMED** `daemon.sh:26` sets `CLAUDE_CODE_TMPDIR=$C/tmp TMPDIR=$C/tmp` and no `CLAUDE_CONFIG_DIR`.
- **CONFIRMED** `/root/.claude/projects` is a volume mounted rw from `devbox_devbox-claude-sessions`; `/root/.claude/.credentials.json` is a symlink to `/run/claude-login/.credentials.json`.
- **INFERRED, settled by R2's control** that landlock, not the volume, is what stops the write. If R2's control passes with `CLAUDE_CONFIG_DIR` unset, the cause is something else and the worker must say so and stop.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| `scripts/checks/resume-through-daemon.sh` | real haiku, private daemon | `RESUME-THROUGH-DAEMON: OK`, control row FAILs as designed |
| `lane-resume.test.sh` empty-tasks case | `$C/tmp/claude-0/p/<sid>/tasks` only | `not found`, no `sid =` line |
| `lane-resume.test.sh` real-jsonl case | `<config dir>/projects/p/<sid>.jsonl` | `RESUMING`, `sid = "<sid>"` |

## Exit Criteria
- [ ] `bash scripts/checks/resume-through-daemon.sh 2>&1 | tail -1 | grep -q "RESUME-THROUGH-DAEMON: OK"`
- [ ] `bash scripts/checks/resume-through-daemon.sh 2>&1 | grep -q "CONTROL.*No conversation found"`
- [ ] `bash scripts/autonomy/tests/lane-resume.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `grep -q 'CLAUDE_CONFIG_DIR' modules/devbox/daemon.sh`
- [ ] `bash scripts/autonomy/tests/lane-preflight.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash scripts/autonomy/tests/lane-graph-select.test.sh 2>&1 | tail -1 | grep -q "FAIL: 0"`
- [ ] `bash -n modules/devbox/daemon.sh`
- [ ] `bash -n scripts/run-batchC-pii.sh`
- [ ] `git diff --quiet -- crates graphs`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Files
- `modules/devbox/daemon.sh`
- `scripts/checks/resume-through-daemon.sh`
- `scripts/run-batchC-pii.sh`
- `scripts/autonomy/tests/lane-resume.test.sh`

## Guardrails
- Do NOT edit `crates/`, `graphs/`, `cycle.sh`, or an existing assertion. Do NOT run `daemon.sh refresh` or restart the production daemon: the check uses its own private daemon. Scratch under `_tmp/` inside the repository.
- The appearance-only implementation: a lane that now looks for `*.jsonl` anywhere but a check that never resumes with a real claude. R2's second step is the proof; a stand-in that prints PONG for any prompt does not count, which is why the second prompt can only be answered from the first run's conversation.
- Mutations the tests must kill: the lane accepting the empty task directory again; the config dir not passed to the private daemon (the control row); the lane looking in the wrong directory; the resume step omitting `sid`.
