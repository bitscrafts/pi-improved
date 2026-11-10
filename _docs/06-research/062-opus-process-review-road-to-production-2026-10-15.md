# 062 — Process review: the road to production (Opus, 2026-10-15)

Read-only review. The only file written is this one; nothing was committed. Labels: **C-run** (I ran it today),
**C-read** (I read the lines), **asserted** (from the brief, not re-checked).

## Verdict

1. **The headline fix did not reach the gate.** `cycle.sh:228` sets `CARGO_TARGET_DIR=_tmp/gate-target`, but
   `xtask/src/fresh_gate.rs:196` sets the child gate's `CARGO_TARGET_DIR` to `<export>/target`, which wins. Only
   xtask's own build sees the warm dir. `_tmp/gate-target` does not exist on disk (C-run, `ls`). **Every gate
   attempt still compiles the workspace cold.**
2. **`lto="fat"` is not on the gate's path.** `quality-gate.sh` has no `--release` step (C-read). It *is* on the
   path of `scripts/checks/deploy-dry-run.sh`, which runs `deploy/install-pii.sh` three times (lines 28, 36, 87),
   and `install-pii.sh:65` is `cargo build --release` (C-read). Spec 152's verify runs that check, so 152 paid a
   fat-LTO release build per verify on top of a cold gate.
3. **A cold gate is minutes, not hours, on this machine when it is alone.** `fresh-gate` measured 5 min 45 s cold
   and idle on 2026-10-11 (059 §1.1, C-run then). The lost day came from **contention**: several cargo consumers
   on one target dir, and one cycle beside another. Contention is the first thing to remove; the cold build is the
   second.
4. **Production has not moved.** `~/.local/bin/pii` is still dated 2026-09-17 01:30 (C-run). Every spec closed
   since then is unshipped. The deploy (M5) is the step with the most value per minute.

## Measured facts used below

| fact | value | label |
|---|---|---|
| Commits since 2026-10-13T15:00 | 68 | C-run |
| of which loop "automated pass" | 8 | C-run |
| of which `autonomy:` process repairs | 15 | C-run |
| of which "leftover" commits (diffs that never passed the gate) | 5; HEAD `b32e549` is one | C-run |
| Spec 152 (A21) | `done`, attempts 11, "verified 4/4 **by hand**", so the loop's acceptance rule was bypassed to close it | C-run, `state.json` |
| Spec 155 (A22) | `verifying`, attempts 3, "plan gate running" | C-run |
| Working tree now | 156's worker edits in the **main** tree (`crates/pii-cli/src/cmd_incidents.rs`, `main.rs`, `pii-learn/src/lib.rs`, two new files) | C-run, `git status` |
| Load average now | 12.96 / 28.30 / 31.29 | C-run |
| Detection of the two self-inflicted outages | ~20 min and 19 min, both by a person | asserted |

## Q1/Q2: make "close a spec" bounded and observable; keep the fresh-tree gate, make it warm

**Keep the design.** The guarantee is that the gate judges the worker's **source**. Build artefacts are a cache,
and caching them does not weaken that guarantee, because cargo fingerprints by content. What must change:

1. **The gate honours a persistent target dir.** In `fresh_gate.rs:196`, use the inherited `CARGO_TARGET_DIR`
   when set, otherwise `<export>/target`.
2. **The export path is fixed.** Use one gate-owned path (for example `_tmp/gate-export`, refreshed in place by
   the gate) instead of a new temp dir per attempt. Workspace crates are fingerprinted by absolute path, so a
   fresh path rebuilds all ~90k lines every time; a fixed path makes a retry incremental. Old exports are moved
   aside, never deleted, per the workspace rule.
3. **The release build leaves the verify path.** `install-pii.sh` gains a profile switch, and
   `deploy-dry-run.sh` uses the dev profile. Fat LTO runs exactly once: at the real deploy.

**Bounded and observable.** Each cycle phase (dispatch, verify, gate) gets a wall-clock budget, and the cycle
writes one status line every 60 s to a single file: phase, elapsed, budget, and the last cargo or worker line.
`health.sh` reads that file. The account meter stops being used as a progress instrument. Over budget means park
with the evidence; it never means a silent wait.

Proof:
- a second gate attempt on an unchanged tree takes **< 25%** of the first's wall time, and its log shows **0**
  `Compiling` lines for registry crates;
- over the next 5 items, dispatch-to-commit-or-park is **≤ 90 min** p90;
- there are **0** minutes in which the lock is held and the status line is older than 120 s.

## Q3: what the orchestrator must be forbidden to do while a worker is in flight, enforced by the machine

1. **No cargo outside the lock.** A `cargo` shim placed first on `PATH` for every shell (orchestrator, loop,
   Hermes jobs) runs `flock -n -E 75 <cycle lock>` around the real cargo. If a cycle holds the lock, it exits 75
   with "a worker holds the build; use health.sh". The loop's own cargo calls already run inside that lock. This
   makes defect (b) impossible. Proof: with a worker live, `cargo build` from the orchestrator's shell returns 75
   in under 1 s, and `ps` shows 0 processes "Blocking waiting for file lock" across a day.
2. **No worker in the main working tree.** The worker writes to a git worktree or export that the loop owns, and
   the main tree is read-only to it. Today 156's edits sit in the main tree (C-run), and a "leftover" becomes a
   **commit on master** (5 in 48 h, C-run). The rule: a leftover is a patch file, never a commit. Proof:
   `git log --format=%s | grep -c leftover` stays at today's count.
3. **No script change goes live without executing once.** An edit to `scripts/autonomy/*.sh` is activated only
   after `bash scripts/autonomy/selftest.sh` and one `DRY=1` cycle pass, run by a pre-activation check rather
   than by hand. Defect (a), a non-executable script that `bash -n` passed, would have failed the `DRY=1`
   execution in the first second. Proof: re-introduce `chmod -x cycle.sh` in a scratch copy, and activation
   refuses.
4. **Nothing is closed by hand.** An item is `done` only with the gate's PASS line in its own cycle log. 152
   (11 attempts, closed "by hand") must be re-gated through the loop once the gate is warm.

## Q4: the smallest pii step that turns "a process dies and the system says so" into a number

Spec 156 (M6, in flight) should pre-register **one number: time-to-notice**, the seconds from a process's death
to an incident line in `pii incidents`.

The test kills three things in a scratch tree and measures each:
- a daemon in the middle of a run;
- a worker;
- a cargo step left to sit cold.

Today's figures are about 20 min and 19 min for the two self-inflicted outages, both noticed by a person
(asserted), and 67+ minutes for the laya stall. **Target: median ≤ 120 s over 3 induced deaths.** A no-op prints
no incident, so the test is red on HEAD by construction.

It depends on 154 (an accepted run is on disk before its first node), or a daemon death leaves nothing to read.
The operator then decides only the end, on the 152 card.

## Q5: what says we should NOT continue on this path as-is

| evidence | value |
|---|---|
| Process versus product in 48 h | 15 loop repairs and 5 ungated leftover commits against 8 loop-made closures |
| Spec 152 | 11 gate attempts, then closed by hand |
| Unshipped work | the deployed binary is still the 2026-09-17 build |
| Self-inflicted outages | two in one day, detected by a person, not by the system |

**Conclusion:** stop growing the loop. Freeze new loop features. Apply changes 1-2 below, then ship (M5).
Re-measure before any new spec.

## Final ranking (at most five changes, each with its proof)

1. **Warm, single-writer gate.** `fresh_gate.rs:196` honours `CARGO_TARGET_DIR`; one fixed export path; the
   release build removed from the dry-run path.
   *Proof:* a second attempt takes < 25% of the first's wall time, with 0 registry `Compiling` lines.
2. **A `cargo` shim behind the cycle lock for every shell, and no worker in the main tree; a leftover is a patch,
   never a commit.**
   *Proof:* the orchestrator's `cargo` exits 75 within 1 s while a worker is live; 0 lock waiters over a day; 0
   new leftover commits.
3. **Ship M5 now, using the existing card (152) and rollback.**
   *Proof:* `~/.local/bin/pii` mtime is after 2026-10-15, `pii health` reports HEAD's revision, and the
   rollback rehearsal has been run once on the host.
4. **Phase budgets plus a 60 s status line; the meter is retired as a progress signal.**
   *Proof:* dispatch-to-commit-or-park p90 ≤ 90 min over 5 items; 0 stale status minutes while the lock is held.
5. **156 pre-registers time-to-notice.**
   *Proof:* median ≤ 120 s over 3 induced deaths (daemon, worker, cold cargo), red on HEAD.
