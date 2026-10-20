# QUEUE BRIEF — 2026-10-20 onward

rows in the plan: 94
  done             89
  skipped          2
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

## Commits: 0 since 2026-10-20
  (since-query returned none — showing the newest commits regardless, because a brief that reports zero
   commits while work landed is worse than no brief)
  09cfe3d 229-podman-shm-lock-and-rootless-reexec.md: automated pass — podman needs /dev/shm for its r
  f8816fa leftover from the previous item, kept for review (cycle 20261020T100015Z; the diff is also a
  ba9b4ea M0 verdict of record on lnx: REFUSED at podman's /dev/shm lock, then its rootless re-exec; s
  347f8f9 M0 script: the verdict quotes the daemon's trace fail= (the log names no run id), and the fi
  d23cb31 M0 script: ship a real python filter beside it for hosts without jq; no nested quoting
  266cf65 M0 script: the jq fallback separates filters from the file argument
  7fdff08 M0 script: the jq fallback reads the file argument jq reads, not only stdin
  2c6a542 M0 script: a jq-less host parses the daemon replies through a python fallback
  df0fdae the sandbox grants podman its state dirs and the image probe quotes the runtime's own stderr
  0d0009a M0 on lnx: three runs refused a present image; sandbox state-dir denials measured with contr
  4867f53 leftover from the previous item, kept for review (cycle 20261020T091514Z; the diff is also a
  908fb0b queue briefs for the 07Z, 08Z and 09Z firings: 92 rows, 87 done, A96 queued with spec 228

## Control and queue state
  == control: RUNNING at 2026-10-19T17:28:22Z (resumed by human)
  rows not done: 5
