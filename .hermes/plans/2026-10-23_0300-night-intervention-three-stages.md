# Night intervention: from "pii runs an external model" to the path to production

Written 2026-10-23 ~03:00Z (00:00 local). Plan only: nothing in here has been executed. Every "state" line is a
measurement from this session, with its run id or commit.

## The three stages (the thing we are searching for)

1. **Stage 1: pii runs an external model**, locally, with installed tools, no podman. pi+ollama and claude+haiku.
2. **Stage 2: pii on the host manages development from within a container.** The coder starts the agent in a
   digest-pinned image with the project mounted; `.pii/.pi/.claude` live in the project, never an auth file.
3. **Stage 3: the path to production.** The deployed daemon runs agent work unattended, bounded, observable,
   restart-safe, quota-safe, for seven days with no unplanned start (070 step 6).

## Where we are (measured, HEAD `050abc2`, gate 14/14)

Stage 1, happy path DONE for both agents:
- pi+ollama-cloud: `r000000000609` rc 0, criterion 1/1, 199 s, no key on argv, `/root/.pi` untouched.
- claude+haiku: `r000000000673` rc 0, 1 turn, $0.0029; `r000000000705` created `hello.txt`=PONG, `tree_changed: yes`;
  `r000000000706` bad model failed at $0.00 and `explain` now names the cause. Login file accepted (`0949e4e`).
- Fault matrix on stand-ins (no spend): fail, flood, secret, orphan, hang+timeout, no credential, zero budget, empty
  brief, route drift. All behave. Fixed: `bcab5a5`, `ea2976f`, `e259bac`, `16236dd`, `4fdfa75`, `0949e4e`, `a31887e`, `050abc2`.

Stage 1, NOT done:
- A failed agent run finishes `succeeded` when the `err` edge ends in a succeeding node (r…641, r…706).
- No-op verdict (7/11): node passes if nothing changed and the criterion already passed. Evidence now exists, policy does not.
- Crash leaves the agent running and the run unresumable (20/21). Cancel is refused on a running node (18). Concurrent
  runs share a workdir and a pi session file (19). The spec path never reaches the prompt.
- Real `claude --resume <sid>` has never been run (the session ids are saved; the resume handle is untested).
- Unexplained: first two stand-in runs took ~43 s end to end (agent 36 ms, tree digest 0.3 s).

Stage 2, BLOCKED on facts, not on code:
- `podman` is not installed where I am (`command not found`). M0 (`_docs/05-operations/M0-2026-10-20.md`) FAILED at the
  `proc` capability, before any podman call. Which machine hosts the container path is not settled.
- `_tmp/pii-agent` (pi's config dir) is a scratch location; it cannot ship. Needs a permanent project-owned home.

Stage 3, open gaps from `071-path-to-production-gaps-2026-10-19.md` plus what this session added:
- Quota behaviour is unmeasured: nothing classifies "spend limit ... resets <time>" and parks the run.
- Gate cache: routine and fresh gates shared a target and failed on stale binaries five times tonight (`68db283` is the fix).
- Restart safety (HARD RULE 0f) untested with an agent in flight on the deployed daemon.

## Guardrails for the night (all standing rules)

- Do NOT touch `/root/.pi`, the deployed daemon (pid 491273), or lnx. Read-only outside the project.
- Queue stays paused (`_tmp/autonomy/control`). Dispatch only from named scripts in `/projects/_tmp/` or `scripts/`.
- No `rm -rf`; move to `_trash`. No commit on a red gate. Neutral identity on every commit. Name files, never `add -A`.
- Real model calls only where listed below; each hard-capped by the graph. No pi/ollama real calls (stand-in covers it).
- Anthropic: quota text => STOP, read the reset time, no retry, no model switch.
- Every check needs a control that fails. Every fix is test-first. Gate green before each commit.
- Stop rule: the same gate failure twice => stop, document, do not loop.

## Night blocks (time-boxed; UTC, local = UTC-3)

### Block 1, 03:00-04:30. Close the Stage 1 defects that need no decision
1a. **Failed agent must not read as success.** First measure: is there a node kind that fails a run? (`grep` the
    node registry.) If yes, point the `err` edge of `claude-probe`, `coder-selffix`, `coder-demo` at it and prove with
    r…641's shape on the stand-in (status `failed`). If no, add a graph-lint warning "coder `err` leads to a
    succeeding terminal" with a near-miss control. Do not change engine semantics (that is your call, option 2).
1b. **Findings doc** `_docs/05-operations/INTERVENTION-2026-10-23-agent-management.md`: defects 1-22, each with run id,
    journal field, status (fixed + commit / open + owner decision). Add OPEN-REGISTER rows for the open ones.
1c. **Cold-start 43 s**: measure only. Script times daemon start -> first run -> first `coder.exited`, three times.
    Record; fix only if the cause is obvious and small.
Exit: gate green; doc committed; open items listed with the decision each needs.

### Block 2, 04:30-05:30. Real resume and quota behaviour (the two Stage 3 unknowns for the paid agent)
2a. **Real `--resume`** (cost ~$0.003, cap $0.10): graph `claude-resume` with `sid` = the saved id of `r…705`
    (`6bb8e279-…`), brief "What file did you create? Reply with its name only." Pass = reply names `hello.txt` AND
    `claude_session_id` unchanged or journalled. Proves the session id is a usable resume handle.
2b. **Quota text on the stand-in**: mode prints "You've hit your monthly spend limit, resets 10:00" rc 1. Measure what
    the journal and run status do today. Expected today: plain failure. Write the red test: the run is parked with the
    reset text journalled, not failed, not retried. Implement only if the park mechanism already exists
    (`run.suspended`); otherwise write the finding and the spec stub.
Exit: 2a PASS/FAIL recorded with ids; 2b measured, test written.

### Block 3, 05:30-07:00. Stage 2 and 3 groundwork that touches nothing live
3a. **Stage 2 preflight script** `scripts/stage2-preflight.sh` (read-only): reports podman present/absent, image digest
    pin, project mount path, `IS_SANDBOX`, which env NAMES the graph forwards, whether `.pii/.pi` exists in the
    project, and the daemon sandbox mode. Run it here; its output becomes the Stage 2 blocker list. Control: pointed
    at a missing path it must report FAIL, not PASS.
3b. **Spec 237 stub** (via spec-author skill): permanent project-owned agent config (`<project>/.pii/.pi/models.json`
    with `$ENV:` key reference, `PI_CODING_AGENT_DIR` set by the coder, never auth). Replaces `_tmp/pii-agent`.
3c. **Production checklist** appended to the findings doc: each Stage 3 requirement, its current evidence, its gap.
Exit: script + stub + checklist committed; nothing deployed.

### Block 4, 07:00-07:30. Morning handoff (what you read first)
Update `HANDOFF.md` and `docs/05-operations/RESUME-REGISTER.md`: state, evidence paths, exact next commands, and the
decisions only you can make. Stand-in daemon and local daemon left running; their pids written down.

## Decisions I will NOT make for you (nothing here is built until you answer)

1. No-op verdict: fail when the tree is unchanged AND the criterion already passed before the agent ran (my recommendation).
2. Where "agent failed => run failed" lives: graph wiring (recommended) or engine (touches every graph).
3. Crash: kill the orphan on restart and make the run resumable, or adopt the orphan.
4. Cancel: allow stopping a running coder node now that `timeout_ms` exists.
5. Agent profile as a graph (neutrality): own file `graphs/agents/<agent>.toml` or a fragment; spec text or path in the prompt.
6. Stage 2: which machine runs podman (this one has none), and whether I may touch it.

## Risks

- A gate red for a stale-binary reason again: use `clean-stale-packages4.py` (one pass, seconds), then rerun.
- The real resume may be refused by claude if the session store differs under `HOME`: record, do not work around.
- Block 2b may show the park mechanism does not exist: then it is a spec, not a night fix.
- Everything is on stand-ins except 2a. A green night proves the harness, not the deployed daemon.
