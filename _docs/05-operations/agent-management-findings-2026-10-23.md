# Findings: managing real agents through pii (pi + ollama-cloud, claude + haiku)

Written 2026-10-23. Every number below was measured in this session from a journal, a test run or a process listing,
except where a line says NOT RUN. Fix commits are given as short hashes; "open" means no commit.

## Stage 1 result

pii managed a real external agent end to end, twice:

- pi + ollama-cloud: run `r000000000609`, rc 0, criterion 1/1, 199 s, route `ollama-cloud/deepseek-v4.1-flash:cloud`,
  no `--api-key` on argv, `/root/.pi` untouched (`269bc39`).
- claude + haiku: write probe `r000000000705` created `hello.txt` (PONG) in 5.3 s, 2 turns, $0.003289, route
  `claude/claude-haiku-5-5`, `tree_changed: yes`. Bad-model probe `r000000000706` failed fast at $0, rc 1, with the
  cause in `explain` (`050abc2`). Total real Anthropic spend: about $0.01 over three calls.

NOT RUN: `claude --resume` on a real session; a real spend-limit refusal; two real runs on one workdir.

## Defects found, in order

Stand-in = `_tmp/fake-agent`, a replaceable `pi`/`claude` with a mode file (no network, no spend).

| # | Defect | Evidence | State |
|---|---|---|---|
| 1-3 | claude arm: no permission mode, no output format, no session id | headless claude could not write; route/session/cost invisible | fixed `16236dd` |
| 4 | claude credential: coder wanted an env token, host login is a file | refused before spawn | fixed `0949e4e` |
| 5 | `haiku` alias unchecked | real call resolved it to `claude-haiku-5-5` | closed by measurement |
| 6 | spec path never reaches the agent prompt | pi found spec 207 by searching the repo | open |
| 7/11 | node passes when the agent changed nothing | `noop` mode succeeded | evidence only (`e259bac`); verdict policy open |
| 8 | empty brief accepted | `brief = ""` ran pi 92 ms, exit 0 | fixed `bcab5a5` |
| 9/10 | `explain` hid the cause of a failure | showed "criteria passed 1/1" beside a failed agent | fixed `050abc2` |
| 12 | no evidence of what the agent changed | `tree_changed` yes/no/unknown, 43,852 files in 0.31 s | fixed `e259bac` |
| 13 | secret printed by the agent reached the journal | token 1x in log, 1x in store | fixed `bcab5a5` (now 0 and 0) |
| 14 | background child outlived the node | `sleep 100000` ppid 1 after "succeeded" | fixed `bcab5a5` |
| 15 | pi `--api-key` on argv | key visible in process listing | fixed `269bc39` |
| 16 | hung agent unstoppable | run `running` at 40 s, cancel refused | fixed `ea2976f` (node `timeout_ms`, rc 124) |
| 17 | validator V10 does not cover `coder` | `coder-selffix` had no `timeout_ms`, nothing complained | open (validator coverage not confirmed) |
| 18 | `run cancel` refuses a running node | `run is executing node 'code'; cancel refused` | open, policy |
| 19 | two runs share one workdir and session file | both ran at once, 204,737 byte session file shared | open |
| 20 | daemon crash leaves the agent running | orphan pid survived SIGKILL of the daemon | open |
| 21 | an interrupted run cannot be resumed | `refused (run already finished: Failed)` | open, policy |
| 22 | a failed agent run can finish `succeeded` | `r000000000641` and `r000000000706`: rc 1, status succeeded | warned by lint V11 (`7111cff`); behaviour open |
| 23 | gate caches share a target, stale export-baked binaries fail tests | repeated `cargo clean -p` | fixed `3217788` and `7a952e7` (queue A104) |

New journal fields from this work: `tree_changed`, `claude_session_id`, `claude_turns`, `claude_cost_usd`,
`failovers`, `route_drift`, `quota_exhausted`, `quota_reset` (`1224d48`, observation only).

## What the evidence says about defect 22

The coder is correct: a non-zero exit goes to the `err` port. The run reads as green because the probe graphs end `err`
in a terminal `transform`. Lint V11 now warns on that shape. It does not change a run's status.

## Decisions that need the operator

1. No-op verdict (7/11). Recommendation: fail only when `tree_changed = no` and the criterion already passed before the
   agent started. Needs one extra criterion run per node.
2. Where "agent failed means run failed" lives (22): in the graph (end `err` in a failing node) or in the engine.
   Recommendation: the graph.
3. Crash (20/21): kill the orphan and mark the run resumable, or adopt it.
4. Cancel (18): may it stop a running coder node, now that `timeout_ms` bounds one.
5. Agent profile as graph data (argv template, credential lookup, output parser) instead of Rust arms: not built.
6. Stage 2 host: which machine runs podman. The deployed daemon runs where there is no podman.

## Known limits

- The installed `pi` is 0.2.0 (`f87d3ff2`); the source read for failover is v0.4.0 (`5bd3e353`). The installed binary has
  no `failover` strings, so a model swap cannot happen on it; the `failovers` field is for the day it is upgraded.
- A background process that left its process group on purpose is not reaped (documented by a test).
- 43 s unexplained delay on the first two stand-in runs; the agent took 36 ms and the digest 0.3 s.
