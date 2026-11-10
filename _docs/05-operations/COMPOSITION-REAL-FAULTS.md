# Composition: real-agent fault scenarios (level 5, continued)

**Date:** 2026-10-27. **HEAD:** `44ba72a588d0`; production devbox daemon at that revision. Real `claude-haiku-5-5` (anthropic) and pi on ollama-cloud.
Run with `scripts/archive/composed-real-faults.sh <cancel|restart|concurrent|wall|impl|pi>`. Each scenario pairs its claim with a control that can fail
(an agent that was ALIVE before the fault, a graph that differs only in the limit, a criterion the work cannot meet).

### Two real composed runs at once

`composed-faults-concurrent-1793093929.log`

```
RUNS r000000000353 r000000000354
PASS two distinct run ids
PASS both runs succeeded (succeeded, succeeded)
PASS four distinct agent sessions
PASS repo a: hello.txt PONG and its own NOTES.md
PASS repo b: hello.txt PONG and its own NOTES.md
PASS nothing committed in either repo
PASS each run's trace has its own call docs
COMPOSED-FAULTS concurrent: 7 pass 0 fail   (log _tmp/claude-batchC/composed-faults-concurrent-1793093929.log)
```

### Cancel while the real docs agent is running

`composed-faults-cancel-1793094104.log`

```
RUN r000000000358
PASS the docs coder (2nd agent) spawned: pid 525905
PASS CONTROL: the docs agent is ALIVE before the cancel (pid 525905)
PASS run.cancelled is journalled for r000000000358 (the store closes a killed cancel as failed)
PASS the real docs agent is GONE after the cancel (pid 525905)
PASS the host's work survived (hello.txt PONG)
PASS the docs step did not complete (no NOTES.md)
PASS run trace: 0 orphans
COMPOSED-FAULTS cancel: 7 pass 0 fail   (log _tmp/claude-batchC/composed-faults-cancel-1793094104.log)
```

### SIGKILL the daemon while the real docs agent is running

`composed-faults-restart-1793094767.log`

```
RUN r000000000361
PASS the docs coder spawned: pid 526741
PASS the pid 524962 IS the pii daemon, not a wrapper
PASS the daemon pid 524962 is dead after SIGKILL
PASS CONTROL: the real docs agent SURVIVED the daemon's SIGKILL (an orphan, pid 526741)
PASS a NEW daemon is running (pid 526819, was 524962)
PASS the run is reconciled to failed (failed)
PASS the docs agent had NOT finished its work (1 of 20 lines)
PASS the restart REAPED the real claude (pid 526741 gone)
COMPOSED-FAULTS restart: 8 pass 0 fail   (log _tmp/claude-batchC/composed-faults-restart-1793094767.log)
```

### A real run past 600 s: the shipped limit against the default

`composed-faults-wall-1793096192.log`

```
RUNS r000000000391 (spec, with the limit)  r000000000392 (specnowall, the default 600 s)
PASS both runs started
A: succeeded after 885390 ms   B: failed after 884643 ms
PASS A (spec, wall limit set) SUCCEEDED
PASS A ran PAST 600 s (885390 ms), so the limit really mattered
PASS A's docs agent did all 40 steps
PASS CONTROL: B (default wall) FAILED
PASS CONTROL: B ran past 600 s before failing (884643 ms)
PASS CONTROL: the two graphs differ ONLY in the limit; both agents exited 0 in B, so the wall limit is what failed it
KNOWN GAP (spec 293, not counted): run explain/get/trace do not name why B failed; the engine records 'max_wall_ms exceeded' and the CLI never prints it
PASS CONTROL: B's host work was complete (hello.txt PONG), the only fault is the limit
COMPOSED-FAULTS wall: 8 pass 0 fail   (log _tmp/claude-batchC/composed-faults-wall-1793096192.log)
```

### A real multi-file implementation, with an unmeetable-criterion control

`composed-faults-impl-1793095888.log`

```
RUNS r000000000389 (four criteria)  r000000000390 (a fifth criterion the work cannot meet)
PASS A: the real implementation run succeeded
PASS A wrote fizz.sh
PASS A wrote check.sh
PASS A wrote README.md
PASS A: the program really prints FizzBuzz (checked here, not by the agent)
PASS A: verify and reverify ran and the run reached done (no unverified)
PASS A: exactly two agent calls (code and the docs coder)
PASS A: spec and expected.txt untouched, nothing committed
PASS CONTROL: B (an unmeetable fifth criterion) FAILED
PASS CONTROL: B's trace reaches the unverified refusal
PASS CONTROL: B was stopped at verify, the docs coder never ran (one agent call)
PASS CONTROL: B's program itself is correct, only the unmeetable criterion failed it
KNOWN GAP (spec 293, not counted): the judge's per-criterion result (4 of 5) is not printed by run trace, get or explain
COMPOSED-FAULTS impl: 12 pass 0 fail   (log _tmp/claude-batchC/composed-faults-impl-1793095888.log)
```

### pi on ollama-cloud through the composed graph

`composed-faults-pi-1793093994.log`

```
RUN r000000000357
PASS the pi/ollama-cloud composed run succeeded
PASS hello.txt PONG written by pi
PASS the docs coder (pi) wrote NOTES.md
PASS two distinct pi sessions (2)
COMPOSED-FAULTS pi: 5 pass 0 fail   (log _tmp/claude-batchC/composed-faults-pi-1793093994.log)
```

## What the scenarios found, including against me

- **My first restart scenario proved nothing.** It killed the `sh -c` wrapper, not the daemon, so the agent finished by itself and the scenario printed
  `REAPED`. Reading the evidence showed the daemon had not died (same pid, run `succeeded`). Fixed: the daemon pid comes from an anchored `pgrep`, must be the
  `pii daemon` binary, must change after the restart, and the agent must still be mid-work (1 of 20 lines) when it is killed.
- **The cancel scenario asserted the wrong word.** `cancel --kill` closes the run as `failed` with reason `cancelled: <reason>` (documented in `runs_cancel.rs`); the
  row now asserts the journalled `run.cancelled`.
- **The real agent refuses a blunt sabotage and follows a plausible one** (see COMPOSITION-REAL-RUN.md).
- **Wall limit, real:** the shipped graph ran 885 638 ms and succeeded; the same graph without the line ran 888 774 ms and failed, with both agents exiting 0.
- **Known gap, not counted as a pass (spec 293):** nothing the CLI prints says WHY the default-wall run failed (`max_wall_ms exceeded` is journalled and never shown), and
  `run trace` shows `crit=-` for a judge node, so a 4-of-5 verdict is invisible.
- **Quota inside a call** is proved with stand-ins that print the CLI's real refusal shape (`scripts/archive/composed-quota-check.sh`, 13 rows): the refused docs coder's
  `coder.exited` carries `quota_exhausted=yes` and the reset text, the host coder's does not, a docs agent that only READ the phrase is not flagged, and
  `is_error:false` is not a refusal. The run ends `failed`; parking the row is `cycle.sh`'s separate decision and is not proved here. A real refusal cannot be provoked on demand.

## Still not proved

- A real quota refusal; a real `--resume` of a composed run; the queue's worker (still a one-node graph, spec 294); pi and ollama under cancel or restart;
  more than two concurrent runs; a real run with a nested call deeper than one level.
