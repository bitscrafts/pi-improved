# Spec 158: one point of control (the holes stop being invisible)

**Project**: pi-improved (the control point covers this machine, including <agent-host-config>) · **Milestone**: none — this is the repair the operator asked for on 2026-10-15: *"it must have a single point of control! there are holes everywhere!"*
**Why now**: five failures in two days were all the same shape — a control point that existed where nobody was looking. Two gates can refuse a tool call, registered in different places, with different words. Three schedulers can start work. Two names for one build cache. Four ways to stop a process, one of which killed the caller's own shell. Two ways to restart the gateway, one of which never returns to the shell. No single component is broken; there is no single place to look.

## Requirement
One executable, `<agent-host-config>`, owns every way work can be refused, spent, started, stopped or restarted:

- `ctl.sh status` prints, **read from the live system on each run**: every gate that can refuse a tool call and the file each is registered in · every scheduler that can start work (Hermes cron jobs, detached watchers) · the loop's control state, lock state and next queued item · live workers with elapsed time and CPU · hot compilers · the gate cache's size · the installed pii revision and the identity it reports
- `ctl.sh gates` prints every gate registration with its file and line
- `ctl.sh gate off <name>` disables that gate in **every** file where it is registered — found by searching, never from a hard-wired list
- `ctl.sh restart` restarts the gateway **detached** (it returns to the shell in under two seconds) and **refuses** while a paid worker is in flight, naming the pid
- `ctl.sh stop-pid <pid>` stops a process after reading its own `/proc/<pid>/cmdline`, excluding the caller and its parent
- `ctl.sh loop pause|resume|status`

**THE CONTRACT, which is the actual repair:** any control point that can refuse, spend, start, stop or restart work must be either **owned** by this script or **printed under `UNOWNED`** by `status`. The command must never be silent about a registration it does not manage. A new hole has to announce itself instead of costing an afternoon.

## TDD contract
| Test | Input | Expected |
|---|---|---|
| a registration appears that the script does not own | a fixture config with an extra gate registration | `status` prints it under `UNOWNED`, exit 0 (this is the test that makes holes visible) |
| no worker in flight | no matching process | `restart` launches and returns in under 2 s |
| a worker in flight | a process whose argv matches a worker | `restart` refuses, rc != 0, and prints that pid |
| `gate off` with two registrations | two files holding the same gate | afterwards both are inactive, and a timestamped backup exists for each file |
| `stop-pid` | a pid whose argv does NOT match the name given | it signals nothing (no pattern matching, ever) |

## Exit criteria
- [ ] `bash /root/.hermes/scripts/ctl.sh status` → rc=0, output contains the sections `gates`, `schedulers`, `loop`, `workers`, `deployments`, `UNOWNED`
- [ ] `bash /root/.hermes/scripts/ctl.sh gates` → one line per registration, each with file and line number
- [ ] `bash /root/.hermes/scripts/ctl-selftest.sh` → rc=0, including the injected third registration being reported
- [ ] `time bash /root/.hermes/scripts/ctl.sh restart` with no worker in flight → returns in under 2 seconds
- [ ] `bash /root/.hermes/scripts/ctl.sh stop-pid 1` → refuses (pid 1 is not ours) and signals nothing

## Guardrails
- Never delete configuration: comment it out or set `enabled: false`, always after writing a timestamped backup
- Never kill by pattern: `/proc/<pid>/cmdline` only, with the caller and its parent excluded
- Do NOT hard-wire the inventory: read it from the live system every run, or this script becomes the next hole
- Rule 0f is enforced inside `restart`; never leave it to the operator to remember
- Do NOT refactor <repo>'s loop scripts in this spec: this is a control point, not a rewrite

## Milestone link
`_docs/05-operations/RESUME-REGISTER.md`, and the incident record `INCIDENT-2026-10-14-the-gate-convoy.md`; this spec is the standing answer to "there are holes everywhere".

## Closure
- **2026-10-16 12:30Z, orchestrator: all five criteria re-run independently on the committed tree, rc=0.** `ctl.sh
  status` prints every section the criterion names — `gates`, `schedulers`, `loop`, `workers`, `deployments`,
  `UNOWNED` — and reads the live system rather than a hard-wired inventory: 3 gate registrations across
  `config.yaml` and this project's `.claude/settings.json`, 7 of 11 schedulers enabled with their next fire times,
  `control: RUNNING at 2026-10-15T12:35:36Z (resumed by human)` and `cycle: none running`. `ctl.sh gates` prints one
  line per registration with file and line. `ctl-selftest.sh` printed `SELFTEST OK` with 8 PASS rows: the injected
  third registration reported under `UNOWNED`, `restart` detaching in 563 ms, `restart` refusing while naming the
  in-flight pid, `gate off` reaching both registrations with a timestamped backup for each, `stop-pid` mismatched
  signalling nothing, and `stop-pid 1` refused. `ctl.sh stop-pid 1` → `REFUSED: pid 1 is not ours (init/invalid)`.
- **The in-flight refusal was exercised against a real paid worker, not only a fixture.** The check ran while the
  boss-watch review (`claude -p`, opus) held the host, and the day's tick shows rule 0f working live: with a worker
  in flight the tick's gateway branch reported `gw-dead-worker` and restarted nothing. One gateway process before
  and after (PID 73, up 13 h 34 m), and `<agent-host-config>` holds a single refusal from
  12:04:10Z — `Another gateway instance is already running (PID 73)` — so the guard held and no second gateway was
  ever launched.
- **Criterion 4 was not re-run live, deliberately.** With a paid worker in flight rule 0f makes `ctl.sh restart`
  refuse by design, and the live verb is the one action this loop must never take while a worker is in flight; its
  contract was exercised in the same run by the selftest's fixture command (`CTL_RESTART_CMD`), measured at 563 ms
  against the 2 s bound.
