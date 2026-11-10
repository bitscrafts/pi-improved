# The progression to production

Written 2026-10-13, after the Opus proactivity judgement (`_docs/06-research/061-…`). One line per milestone, its
gate, and who does it. The rule the whole thing obeys: **the operator stays as far from the loop as possible and
decides only the end** — so every milestone below is either a machine act or exactly one decision for a person.

## Where we are (measured, not assumed)

- The tree is green and the builder loop closes specs unattended (133→149 done, each on its own `fresh-gate`).
- `deploy/install-pii.sh` and `deploy/rollback-pii.sh` exist (spec 142) and have **never been run**.
- The installed `pii` on the machine dates from 2026-09-17 (059): **86 commits behind**; no daemon runs.
- The pii runtime logs the reviewer wants a reader for **do not exist yet**, because nothing is deployed.

## The milestones

| # | milestone | gate (red before it) | who |
|---|---|---|---|
| **M0** | The builder loop is trustworthy enough to be left alone | `bash scripts/autonomy/selftest.sh` → ALL PASS (27 today); `window.sh` honours the reset the CLI names; a cut-off worker keeps its resume handle | machine ✓ (done) |
| **M1** | **Detection**: an incident is opened from our own records, no model (spec 150) | replay the preserved cycle log → exactly **1** `dirty-tree` incident (not 497), 1 `verify-failed`, 0 benign; then, live, one incident line reaches the phone within a day | machine; needs 153 first |
| **M2** | **Evidence on a crash**: an accepted run is on disk before its first node (spec 154) | `kill -9` the daemon 2 s in → `pii run get` answers terminal `failed`, reason `interrupted by daemon restart`, and one reconcile event exists; HEAD answers "unknown" | machine |
| **M3** | **Resolution**: an incident becomes a proven fix on a branch, never on `master` (spec 151) | the loop itself shows the named test **red on base, green on the branch** + `fresh-gate`; **pre-registered decision**: build 151 only if **≥3 of 7** historical failed cycles produce a mechanically proven fix at < $1 each | machine |
| **M4** | **One permission**: the deploy card (spec 152) | five fields (incident+count · diff+branch · proving test's two runs · `fresh-gate` PASS · dry-run rollback); removing **any one** refuses the card (5 reds); `DEPLOY_DRY=1` card → `yes` prints the exact rev and the rollback | machine |
| **M5** | **Production** — the first real deploy | `pii health` identity == the deployed rev · the unit survives 24 h on the host · `pii_channel_posts_total ≥ 7` over 7 days · Hermes' call and error rates unchanged (±50% of the 161/30 d baseline) | **you**: place the token, enable the unit; the loop does the install and the rollback is one command |
| **M6** | pii reads its **own** runtime logs, not just the builder's | the same incident rule (150) over the deployed daemon's `run.start.refused`, store and panic events; a real incident from production records within a week | machine |
| **M7** | Retire the dormant surface (059 §2.6) | the six decisions: keep-or-retire `gemini`, telegram/matrix/webhook, the `gpiod` HAL, the local `ollama.service`, the forum-as-home switch | **you** |

## The order, and why

`153 → 150 → 154 → 151 → 152 → M5 → M6 → M7`. 153 first because it protects the measurement (a metric fed by test
noise is not a metric). 150 next because it attacks the **41.5-hour detection gap** the judgement measured, and it
needs no model. 154 then, because until a crash leaves evidence an incident cannot even see it. 151 only if the
pre-registered experiment earns it. 152 last, because a permission card for a deploy nobody has exercised is
theatre until there is a change worth shipping.

## The one decision that is yours, and when it comes

**M5.** Everything before it is reversible by construction (branches, records, cards). M5 is the first act that
changes what runs on your machine, and the card is designed to make it a five-second decision: the incident, the
diff, both runs of the proving test, the gate's verdict, and the rollback already dry-run. Nothing else in this
progression asks you for anything.

## What would make us stop

- If the 7-cycle experiment proves fewer than 3 fixes, 151 is not built and a person resolves from the evidence
  (150 + 152 still stand).
- If `pii health` cannot prove the deployed revision, M5 is rolled back and the deploy script is the defect.
- If the deployed daemon produces no incident-worthy records in a week, M6 is premature and 150's sources get
  revisited rather than widened "just in case".

## Measured: the runtime reader on the production log (spec 160, 2026-10-16)

`pii incidents scan --source runtime --log <copy of production pii.log>`: starts seen 1 of 1 (grep of
`daemon.started` agrees), false incidents 0 (the audit log copy: 0, was 5), time-to-notice = 22 ms (start
marker to `daemon.started`). Rotated siblings `pii.log.N` are read with the current file.

## 0.2.0 is live (2026-11-03)

`pii 0.2.0 (085369902766, x86_64-unknown-linux-gnu)` on `the host`, `systemd --user`, active from 02:17:51Z, client and
daemon agree, seven stores intact, `HOST-CONFORMANCE: 5 passed, 0 failed`. Card: `_tmp/deploy/card-085369902766.json`.

The release gate ran the FULL list — 20 steps — and the six runs it took to get there are the record of why the
quick list is not the gate: it found five real defects, one of them a channel backoff that saturated at 16 seconds
and could never reach its five-minute ceiling. Two of the five were made by this agent in the same night.

Order that mattered: gate the COMMITTED revision in a clone → release build stamped with that revision → card →
aside copy of the previous binary (the installer overwrites it in place; it keeps stores, not binaries) → install →
the five host checks. The rollback is a copy of `~/.local/bin/pii.rollback-177e4efbcf07` plus a restart.
