# QUEUE BRIEF — 2026-10-20 onward

rows in the plan: 99
  done             93
  skipped          2
  needs-review     2
  revoked          1
  parked           1

## Closed: 93 rows
## NOT done — kept visible, never summarised away
  A102  [needs-review]  234-a-skill-edit-goes-through-the-daemon.md  gate-failed
  A65  [skipped]  201-a-cancelled-run-does-not-fire.md  duplicate of A63/196: remove_run + cancel disarm shipped in 5a6445e and exercised on a scratch daemo
  A71  [skipped]  144-a-judge-node-bounded-judgment-per-row.md  spec 144 is marked SUPERSEDED / NOT QUEUED FOR IMPLEMENTATION: the worker refused to build it and wa
  A79  [revoked]  211-hooks-are-documents-you-can-audit.md  REVOKED by review 076 §4.6: hooks are attaches (spec 212 / row A80, done), not a separate document k
  A94  [parked]  225-project-scaffold-and-per-run-skills.md  parked on M0 and on an unwired seam: 5/6 criteria green, but criterion 6 needs a spawn and M0 measur
  A95  [needs-review]  227-the-coder-gets-its-skills-from-the-store.md  awaiting measurement: spec 233 (3d32079) makes its criterion 7 runnable; the unit criteria 1-6 stay 

## Commits: 0 since 2026-10-20
  (since-query returned none — showing the newest commits regardless, because a brief that reports zero
   commits while work landed is worse than no brief)
  622826e 234: a live skill's text can be edited through the daemon - skill.edit keeps the document id
  541cdbd 233: the tree the worker left is gated - skill.edit through the daemon's store (spec 234) wi
  f50f970 gate: coder_runtime splits by transport edge - the podman CLI runtime (spec 209) and the HTT
  4ce1345 gate: dispatch.rs keeps the dispatch, the in-flight barrier moves to sched/barrier.rs - 437 
  afe021d 221: the run's verdict folds its own journal - judged() reads the durable prefix plus the bu
  171b7ae leftover from the previous item, kept for review (cycle 20261020T142830Z; the diff is also a
  a553a3b queue: 233 filed with the live readings (zero skills in the store, the lock, the wrong defau
  3d32079 233: a skill gets into the daemon's store through the daemon - skill.put/skill.list on the e
  035d073 leftover from the previous item, kept for review (cycle 20261020T120419Z; the diff is also a
  09d1174 233: a skill can be added through the daemon - spec 227 criterion 7 is unreachable on the host wi
  8d77adf 232: the workspace criterion is scope-checked - cargo:test:workspace/all resolves only when 
  f175423 the the host deploy restart stops the real daemon: the proc cmdline string-match killed the bash 

## Control and queue state
  == control: RUNNING at 2026-10-19T17:28:22Z (resumed by human)
  rows not done: 6
