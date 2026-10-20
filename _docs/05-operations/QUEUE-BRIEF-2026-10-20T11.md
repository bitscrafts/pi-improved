# QUEUE BRIEF — 2026-10-20 onward

rows in the plan: 96
  done             89
  skipped          2
  -                2
  revoked          1
  parked           1
  needs-review     1

## Closed: 89 rows
## NOT done — kept visible, never summarised away
  A65  [skipped]  201-a-cancelled-run-does-not-fire.md  duplicate of A63/196: remove_run + cancel disarm shipped in 5a6445e and exercised on a scratch daemo
  A71  [skipped]  144-a-judge-node-bounded-judgment-per-row.md  spec 144 is marked SUPERSEDED / NOT QUEUED FOR IMPLEMENTATION: the worker refused to build it and wa
  A79  [revoked]  211-hooks-are-documents-you-can-audit.md  REVOKED by review 076 §4.6: hooks are attaches (spec 212 / row A80, done), not a separate document k
  A94  [parked]  225-project-scaffold-and-per-run-skills.md  parked on M0 and on an unwired seam: 5/6 criteria green, but criterion 6 needs a spawn and M0 measur
  A95  [needs-review]  227-the-coder-gets-its-skills-from-the-store.md  six unit criteria green by the orchestrator on the committed tree (live_registry_supplies_the_skill_
  A98  [-]  230-the-coder-starts-containers-through-the-runtime-api.md  
  A99  [-]  231-trace-tells-the-truth-about-a-refusal.md  

## Commits: 0 since 2026-10-20
  (since-query returned none — showing the newest commits regardless, because a brief that reports zero
   commits while work landed is worse than no brief)
  4fc1932 M0 measured against review 078's predictions: the direct path dies in the landlock domain, a
  1737a1c M0 live on lnx reached only the /dev/shm gate: the deployed build predates spec 229, and its
  3b4a209 sandbox-exec applies the same container state dirs as the daemon, so the workspace profile m
  e5fe628 M0 on lnx: the third gate is podman's user-namespace setup, and a confined process starts a 
  09cfe3d 229-podman-shm-lock-and-rootless-reexec.md: automated pass — podman needs /dev/shm for its r
  f8816fa leftover from the previous item, kept for review (cycle 20261020T100015Z; the diff is also a
  ba9b4ea M0 verdict of record on lnx: REFUSED at podman's /dev/shm lock, then its rootless re-exec; s
  347f8f9 M0 script: the verdict quotes the daemon's trace fail= (the log names no run id), and the fi
  d23cb31 M0 script: ship a real python filter beside it for hosts without jq; no nested quoting
  266cf65 M0 script: the jq fallback separates filters from the file argument
  7fdff08 M0 script: the jq fallback reads the file argument jq reads, not only stdin
  2c6a542 M0 script: a jq-less host parses the daemon replies through a python fallback

## Control and queue state
  == control: RUNNING at 2026-10-19T17:28:22Z (resumed by human)
  rows not done: 7

## This session (2026-10-20, 10:30-11:50Z) — measured, not summarised

**Rows closed: 0.** Two rows were added (A98, A99), and one already-done row's spec was completed.

| commit | what it closed |
|---|---|
| `e5fe628` | M0's third gate measured + spec 230 + row A98 |
| `3b4a209` | spec 229's CLI half: `sandbox-exec` now applies the same podman state dirs as the daemon (A97's spec is complete in both places) |
| `1737a1c` | live M0 on lnx (r210/r211) + spec 231 + row A99 + the script marks a clipped reason |
| `4fc1932` | the measurement against review 078's predictions |

**M0's verdict, live in this session (two runs):**

    run=r000000000210  M0 REFUSED Error: failed to open 2048 locks in /libpod_rootless_lock_1000: permission denie
    run=r000000000211  M0 REFUSED … same, now marked [clipped-80: trace cuts fail=; full reason pending A99]

That gate is the one spec 229 fixes: the deployed lnx daemon is `pii 0.1.0 (df0fdae16366)`, and its live
`grant_policy` names only spec 228's three state dirs. Nothing from 229 onward is installed.

The deeper gate, measured with the grant replica (must-fail probe `RESTRICT OK` on every call) and a
control: the daemon grant cannot set up rootless podman's user namespace — `newuidmap: write to uid_map
failed: Operation not permitted`, rc 125 with a fresh state; `cannot re-exec process to join the existing
user namespace`, rc 1 with the existing one; control, no confinement: the image runs, rc 0. Granting
`/proc` read-write moves the refusal from an open denial to that write denial, so **no grant buys it**.

The way through, measured: from inside the same enforced grant, `podman.socket` answered and the image
started — create → id, start 204, wait 0, `ExitCode 0`, delete 200. Spec 230 / row A98 make the coder
node speak that protocol.

**Did any coder run spawn an agent? NO.** No daemon on lnx has journalled `coder.spawned` for a container
path; both live runs say `trace: 2 nodes · 0 agent calls`. Job 2 (the coder demo on `/tmp/pii-ws-3aQWp1`)
is blocked, and nothing about pii managing coding agents, the container design, or production readiness may
be claimed.

**Still open, with the reason:**

- `A98` (spec 230) — queued and dispatchable; the dispatcher cron (`*/5 * * * *`, enabled) picks it.
- `A99` (spec 231) — queued and dispatchable. Both were added because a finding may not live only in a report.
- `A94` (spec 225) — parked: criterion 6 needs a spawn, which needs M0.
- `A95` (spec 227) — needs-review: six unit criteria green; criterion 7 (live lnx spawn with the pin) is
  unmeasurable while M0 refuses. Both unpark on M0 PASS.
