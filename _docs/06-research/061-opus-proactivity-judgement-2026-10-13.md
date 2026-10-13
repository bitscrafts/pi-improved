# 061 — The proactivity judgement: detection first, one permission at the end (Opus, 2026-10-13)

**PROVENANCE.** An opinion, not an authorisation. Written by `claude-opus-5-5` (effort high, test mode, read-only)
in a resumed review session (`ef17e6d6-1741-42d6-87c6-16ffc775d231`, the reviewer of 043/045/059), 3 turns after
the account window cut the same session at turn 30. Run: `exit=0`, `stop_reason end_turn`, session cost
`$56.73`, tree clean (0 changed paths). It **contradicts two specs I wrote (143, 144) with measurements** and
proposes a different set. Nothing here is implemented until it has its own spec and its own pre-registered gate.

**The one-line verdict:** our detection latency is measured in **days**, not seconds; a fixed model-free rule over
the loop's own log found **2 of 2 real incidents** that a person then fixed hours later, so `143` and `144` are
both dead as written and the first thing to build is an **incident record + one notification**, then a proven fix
on a branch, then **one permission** (the deploy card).

**Measured contradictions to my own drafts (all C-run/C-read by the reviewer):**
- `reconcile_boot` (`crates/pii-server/src/engine/session_durability.rs:300-324`, called at `cmd_daemon.rs:417`)
  already marks `Running` runs `Failed` — so 143's premise was false; and the killed-daemon crash it tested left
  **no record at all** because the journal buffers 16 events before writing (`pii-graph/src/journal.rs:45`).
- 143's four criteria pass on unchanged HEAD; 144's three do, and its fourth (`deps-check | tail -1 | grep -qv FAIL`)
  can never pass, because `deps-check` prints nothing on success.
- Every incident on this machine is an exact-match string (551 rows, 11 distinct values) — a model judge buys
  nothing yet; revisit when >20% of incident-worthy rows lack a stable fingerprint.
- The measured failure the user lived through: a `dirty-tree` incident opened at +10 min by the fixed rule, but a
  person found it **41.5 h later** after **497** identical refusals.
- Operator-distance baseline from git: since 2026-10-11T15:00, **44 commits — 10 by the loop, 34 by hand, 17 of
  those repairs to the loop itself ≈ 3.4 hand-made commits per spec closed**.
- Correction of my brief's size numbers: 24 workspace members, 400 tracked `.rs`, 90,495 lines, 1,126 test
  attributes (mine: 25 / 408 / ~91,024 / 1,103 — the inventory script counts differently).

**Its proposed order:** 153 (the operator's records are not test fixtures) → 150 (an incident from the system's
own records, no model) → the durability item (an accepted run on disk before its first node) → 151 (an incident to
a proven fix on a branch) → 152 (one permission: the deploy card). Full text below, unedited.

---

The replay settles detection (C-run). The fixed rule turns 551 recorded outcomes (11 distinct values) into exactly 3 incidents:

| incident | first seen | opened | later outcome |
|---|---|---|---|
| `window-closed` | 10-11 17:48 | +7 min | benign quota closures |
| `dirty-tree` | 10-11 21:01 | +10 min | a person fixed it about 41.5 h later |
| `verify-failed` | 10-13 14:44 | +5 min | a person corrected the spec 141 criterion in commit `fd62a20` |

Two of the three are real problems that a person fixed, and neither needed a model to detect.

# Review: proactivity as pii's self-improvement, and specs 143/144 (2026-10-13)

The repo was not edited and nothing was committed (`git status` showed 0 changed paths after my runs). Scratch work is under `/projects/_tmp/pii-review-20261013-*`. I did not touch the loop's state files. That is also why I did not run its selftest: it writes into them (Part 4).

**Evidence labels:**

| label | meaning |
|---|---|
| **C-run** | I ran it this session |
| **C-read** | I read the cited lines |
| **I** | inferred |
| **asserted** | stated in the brief or a commit message and not re-checked |

## Part 1: the verdict on the thesis

**The user's reading is right in direction, and the measurements say where the value is.** On this machine, the expensive failures were noticed late; fixing them was never the hard part. A fixed, model-free rule over the loop's own log would have opened the `dirty-tree` incident 10 minutes after it began. A person found it about **41.5 hours** later, after **497** identical refusals (C-run, replay above). Commit `26c3c89` records a second case: the gateway was dead and "no message reached the phone for two days" (asserted). The video's number that matters is detection at T+5.8 s. Ours is measured in days. **Build detection and one notification first, then resolution on a branch, then the one permission.** In that order, every step shows its value before the next costs anything.

**Where the reading is incomplete for pii, which today has one self-action (a graph quarantine):**

1. **There are no pii production logs to read yet.** pii is not deployed. `/root/.local/bin/pii` dates from 2026-09-17 and `pii-mcp` from 2026-09-09, and no daemon runs (C-run). The deploy script from spec 142 exists (`deploy/install-pii.sh`) and has never been run on the host. The real incidents today live in the builder loop's records and the gate's output. The thesis has to start there, and add pii's runtime logs after the first deploy.
2. **"One permission at the end" presumes a production to permit into.** That end gate, `install-pii.sh` together with `rollback-pii.sh`, has been exercised zero times. Until it has been, "the operator decides only the end" describes a decision nobody has ever been asked to make.
3. **A quarantine is a production effect, so it cannot be a self-action.** It stops a graph from running for its users. It must sit behind the permission, as it does today: the learn timer has no path to the promoter (`crates/pii-server/src/daemon/learn_loop.rs:1-4`).
4. **The operator's only switch is not trustworthy today.** The brief says the control file is deliberately PAUSED. It reads `RUNNING at 2026-10-13T15:13:00Z (resumed by human)` (C-run). Before commit `4cccdd7` (15:13:43), `control.sh` ignored `PII_CONTROL` (C-read, the diff of `4cccdd7`), so every selftest run paused and then resumed the real switch. Selftest runs left notes at 15:07, 15:08, 15:10 and 15:13 (`_tmp/autonomy/notes.md`). Even after the fix, the selftest still writes the real `questions.md`, `notes.md` and `state.json`. Its assertion at `scripts/autonomy/selftest.sh:66` also reads the real control file rather than `$PII_CONTROL` (C-read). An operator-distance metric built on these records would count test noise as operator acts.
5. **Size (about 6x Pi Durable's 15k-line budget) matters less than the brief fears.** Measured, there are 63,273 lines in `crates/*/src` and 90,495 in all tracked `.rs` files (C-run). The proving-test discipline confines the agent to a failing test, a fingerprint and a diff, not the whole harness. Scope by fingerprint, not by shrinking the codebase.

### The four beats

| beat | exists (file:line) | missing |
|---|---|---|
| **1. Reads its own logs** | `pii-learn` scans only `<root>/trajectories/*.jsonl` (`crates/pii-learn/src/scan.rs:242-255`). A daily learn timer exists (`daemon/learn_loop.rs`, wired at `daemon/mod.rs:224-227`) but returns early without a trajectory root (`:226`); trajectories are off by default, and `deploy/install-pii.sh:93` turns them on. The builder loop writes one `OUTCOME=` line per cycle (`scripts/autonomy/cycle.sh`, for example lines 103 and 85 of its second half) to `_tmp/autonomy/logs/`. `fresh-gate` keeps failing output at `<export>/gate-output.log`. | **No reader of** the loop's `OUTCOME` lines, the gate's failing-test names, or the daemon's log and audit events (`grep` over `crates/pii-learn/src` finds no `filelog`, `audit.log` or `gate-output`, C-run). There is no incident record and no fingerprint. pii's own runtime failures do not exist on this machine because nothing is deployed. |
| **2. Resolves by itself** | `quarantine_graph` is the only action (`crates/pii-learn/src/promote/`). Promotion is a daemon RPC with one writer since 138 (`crates/pii-proto/src/method.rs:112-116`). Shadow-run verification (`crates/pii-learn/src/verify.rs`). **The real actuator already exists in the builder loop:** it dispatches a worker, runs the item's criteria, runs `fresh-gate`, then commits or parks (`scripts/autonomy/cycle.sh` steps 3-6). | No path from an incident to a brief. No branch: the loop commits straight to `master`. No proving-test check, meaning red on the base revision and green on the fix, run by the loop rather than reported by the worker. |
| **3. Asks permission** | `approval.redb` with `put_approval_if_absent` (`crates/pii-server/src/daemon/approvals.rs:48-70`); the refusal decision ledger (`crates/pii-server/src/engine/refusal.rs`); learn promotions (human-only); `control.sh ask`/`answer` → `questions.md` → the tick carries it to the phone (`scripts/autonomy/control.sh:42-45`); `deploy/install-pii.sh` and `rollback-pii.sh`. | No single approval artefact carrying the diff, proof and rollback. The deploy has never been asked for or run. Two questions in `questions.md` (14:54:34 and 14:59:40) are still unanswered. |
| **4. Operator distance** | Nothing measures it. | Baseline measured from git, separating by message shape because loop and orchestrator commit under one author identity by rule: since 2026-10-11T15:00 there are **44 commits, of which 10 were made by the loop and 34 by hand, 17 of those 34 being `autonomy:` repairs to the loop itself** (C-run). That is about **3.4 hand-made commits per spec closed**. |

## Part 2: the spec set

**Numbering.** Commit `ea6c1fe`'s neighbours include a commit titled "specs 145-148", so I use 149-153 (asserted; renumber if any is taken). Every gate below is red on unchanged HEAD by construction, which is the user's "no criterion a no-op satisfies" rule.

### 143: drop it as written (Q6, Q9)

Its premise is wrong twice, and all four of its criteria pass with no change.

| 143's claim | measurement | label |
|---|---|---|
| "nothing on start-up looks for" a run left `Running` | `reconcile_boot` (`crates/pii-server/src/engine/session_durability.rs:300-324`) marks every `Running` run `Failed` with "interrupted by daemon restart". It is called at daemon start (`crates/pii-cli/src/cmd_daemon.rs:417`) and tested (`crates/pii-server/tests/session.rs:142`). It never auto-resumes, by decision G8. Its write result is discarded with `let _ =` (`:315`). | C-read |
| "pii persists run records; it does not persist the step" | Steps are journalled (`NodeStarted`/`NodeFinished`, `crates/pii-abi/src/journal.rs:80-87`). A resume already marks a node left mid-flight with `NodeInterrupted` (`journal.rs:122-129`), tested in `crates/pii-graph/tests/resume.rs:177-185`. | C-read |
| "stays `Running` for ever" | Killed with `kill -9` 1 s and again 15 s into a 30 s node, then restarted: `run 'r000000000001' is unknown to this daemon (not in flight, not stored)`. The run never reached the store because the journal buffers up to **16 events** before writing (`DEFAULT_BATCH`, `crates/pii-graph/src/journal.rs:45`). The real defect is a crash that leaves **no record at all**, not a zombie. | C-run |
| its criteria test the change | On unchanged HEAD all four pass: pii-server lib, pii-store, clippy, and `grep Interrupted` (already present at `journal.rs:127`). | C-run |
| the held implementation | It sweeps `runs_by_status(Running)` (`recovery.rs:45` on the held branch). That set is empty in the measured crash, so it would pass its own tests and miss the real case. | C-read |

**What survives, as 149, is the only durability item the thesis needs: a crash must leave evidence a log reader can see.**

> **149 · An accepted run is on disk before its first node runs.**
> - **R1** At acceptance, write the run header (`Running`) and its `Started` event synchronously, outside the 16-event batch.
> - **R2** `reconcile_boot` handles its write error instead of `let _ =`, and emits one event per reconciled run.
>
> **Gate (pre-registered):** in a scratch export, `pii run start starter/slow`; `kill -9` the daemon at 2 s; restart; `pii run get <id>`.
> - Must answer a terminal `failed` with reason `interrupted by daemon restart`, and the daemon log must hold one reconcile event.
> - **HEAD answers "unknown", so a no-op is red.** Plus `fresh-gate`.
>
> **Effect on distance:** none directly. Without it, a process crash never becomes an incident, so it is a precondition for beat 1.

### 144: keep it as research, not next (Q6, Q8, Q9)

What is wrong with it:
- **Its criteria.** 3 of 4 pass on unchanged HEAD (pii-server lib, pii-graph, clippy, C-run). The fourth, `cargo run -q -p xtask -- deps-check 2>&1 | tail -1 | grep -qv 'FAIL'`, **can never pass on a correct tree**. `deps-check` prints 0 lines on success (rc 0), and `grep -qv` on empty input returns 1 (C-run). It also checks crate dependencies, not the "example graph" it claims to validate.
- **Its premises.** "Costs nothing per row" is false: local compute is not free, and no per-row latency was measured for any model. A "calibrated confidence" has no criterion that tests calibration. The cron facility it attributes to spec 128 is F5.7 (`crates/pii-graph/src/cron.rs`).
- **The decisive fact.** Every incident measured on this machine is an **exact-match string**: 551 outcomes across 11 distinct values; the gate's failing-test names; `run.start.refused`. A deterministic rule found 2 of 2 real incidents (C-run). A model judge adds cost and a failure mode before any row has needed judgment. Rule-based routing already exists in the `classify` node (`crates/pii-server/src/node_kinds/nodes/classify.rs`).
- **When to revisit:** when a stream exists in which **more than 20% of incident-worthy rows have no stable fingerprint**, measured on a week of real pii daemon logs.

### The specs the thesis needs

| id | title | smallest requirements that make it real | pre-registered gate (red on HEAD) | effect on operator distance |
|---|---|---|---|---|
| **150 (first)** | **An incident is opened from the system's own records, with no model** | R1 a `pii-learn` source beside trajectories reads `OUTCOME=` lines from the loop's logs, the failing-test names in `gate-output.log`, and the daemon's `run.start.refused`, store and panic events, each as a row (ts, source, fingerprint, evidence path:line). R2 the threshold lives in config (default N=3 within 30 min) with a list of known-benign fingerprints (`window-closed`). R3 one incident per fingerprint per window, appended to `incidents.jsonl` with `first_seen`, `opened_at`, `count` and evidence. R4 `tick.sh` (already the no-agent cron) runs it and sends one phone line per new incident. | Replay a copy of `_tmp/autonomy/logs/cycle-last-detached.log` as a fixture: **exactly one** `dirty-tree` incident opened at 2026-10-11T21:11 (not 497), one `verify-failed` at 2026-10-13T14:49, and **zero** for `window-closed` and `done`. A no-op opens zero, so it is red. Plus `fresh-gate`. | Detection latency on the measured incident drops from **41.5 h to 10 min**. A person's first act moves from "notice" to "read a phone line". |
| **151** | **The loop takes an incident to a proven fix on a branch** | R1 `cycle.sh` gains a second plan source: an open incident becomes a brief in `_tmp/autonomy/briefs/` (never `specs/`), built from its evidence. R2 the worker works on branch `incident/<fingerprint>`, never `master`. R3 the loop itself runs the named proving test: red on the base revision, green on the branch, plus `fresh-gate` on the branch. Any leg missing means park with the evidence. R4 budget cap: 1 dispatch per fingerprint per day and a cost ceiling per incident. | Seed a scratch tree at the parent of `9fc7f9f` (the spec 133 fix of the quarantine-test race) with an incident "test `a_quarantined_graph_is_refused…` failed": the loop must leave a branch whose named test is red on base and green on the branch, and a card. A no-op leaves no branch, so it is red. | Hand actions before the permission become **0** for the "failing test" class. |
| **152** | **One permission: the deploy card** | R1 one artefact per change with five fields: incident (fingerprint, first and last seen, count); diff stat and branch; proving test with both runs (exit codes and log lines); `fresh-gate` result; rollback command (`deploy/rollback-pii.sh <prev-rev>`) already dry-run. R2 a card missing any field is refused, never shown. R3 `answer yes` is the only path to `install-pii.sh <rev>` (a host action); `no` parks with the reason. R4 one card means one change, never a bundle. | Unit: each of the five fields removed in turn refuses the card (5 reds). End to end: `DEPLOY_DRY=1` card → yes → the dry install prints the exact rev and rollback. HEAD has no card, so it is red. | The operator's acts per incident become **exactly 1**. |
| **153** | **The operator's records are not test fixtures** | R1 `control.sh` paths for `questions.md`, `notes.md` and `state.json` become overridable like `PII_CONTROL`. R2 the selftest exports all of them and its assertion at `selftest.sh:66` reads `$PII_CONTROL`. R3 every operator act (`pause`, `resume`, `answer`, `skip`) appends a timestamped line to an `operator-acts.log`. | `sha256sum` of the real `control`, `questions.md`, `notes.md` and `state.json` before and after `bash scripts/autonomy/selftest.sh` must be identical. **On HEAD they are not, since the selftest appends to `notes.md` and `questions.md` (C-read), so a no-op is red.** | Makes the metric below measurable; without it, test noise counts as operator acts. |
| **149** | An accepted run is on disk before its first node runs | see above | see above | precondition for beat 1 once pii is deployed |

**Order: 153 → 150 → 149 → 151 → 152.** 153 is about an hour of work and protects the measurement. 150 is the first real spec: it attacks the measured 41.5-hour failure and needs no model.

### Q3: the minimum self-actions

| action | without a person? | why |
|---|---|---|
| file an incident or decision record | **yes** | append-only; the record is the evidence |
| re-run a failed gate once, in a fresh export | **yes**, at most once per fingerprint per hour | changes no tree; the new log is the evidence |
| open a run on a **pre-registered** diagnostic graph with read-only caps | **yes** | capped by the graph; no `fs_write`, no `net` |
| draft a brief into `_tmp/autonomy/briefs/` | **yes** | not `specs/` (planner-only); a brief changes nothing |
| open a branch with a fix and a proving test | **yes** | the branch is reversible and `master` is untouched; proof is checked by the loop, not reported by the worker |
| merge a proven, gated branch into `master` | **yes, while `master` ≠ production** | true today (the deploy is separate); the builder loop already does this after `fresh-gate` |
| **quarantine a graph** | **no**, a card | it changes what users can run |
| **deploy (`install-pii.sh`)** | **no**: the one permission | it is production |
| edit `specs/`, widen egress or grants, touch secrets, delete, migrate a store | **never automated** | irreversible, or it widens capability |

### Q4: the operator-distance metric

**Definition.** For each incident in `incidents.jsonl` (spec 150):
- `H_pre`: the number of `operator-acts.log` lines (spec 153) between `first_seen` and the card's `asked_at`;
- `L`: `opened_at − first_seen` (detection latency);
- `A`: `asked_at − first_seen` (time to the permission request).

**Primary measure:** the share of incidents in a 14-day window that reached a card with `H_pre = 0`.

**Honest targets for a two-person project:**
- at least 50% of incidents reach a card with `H_pre = 0`;
- median `L` ≤ 30 min (one tick), against a measured 41.5 h;
- at most 1 operator act per resolved incident.

**Command once 150 and 153 exist** (pre-registered):

```bash
jq -s '[.[] | select(.asked_at)] as $a
       | {incidents: length, carded: ($a|length),
          zero_touch: ([$a[] | select(.h_pre==0)] | length)}' _tmp/autonomy/incidents.jsonl
```

**Today's baseline, from git:**

```bash
git log --since=2026-10-11T15:00 --format=%s | grep -c 'automated pass\|leftover from the previous item'   # 10
git log --since=2026-10-11T15:00 --oneline | wc -l                                                          # 44
```

### Q5: the one permission

The permission is **"deploy revision X to the host"**. The card (spec 152) must carry five fields:
1. the incident, with fingerprint and count;
2. the diff stat;
3. the proving test's red-on-base and green-on-branch exit codes;
4. `fresh-gate` PASS;
5. the dry-run rollback command.

That makes "yes" a five-second decision.

**Never behind a "yes":**
- a change without a proving test;
- a bundle of unrelated changes;
- anything widening grants or channels;
- secrets, deletions, store migrations;
- edits to `specs/`.

The three existing records keep their jobs: `approval.redb` for graph gates, the decision ledger for refusals, `questions.md` for the phone. The card is one new artefact type, not a fourth ledger.

### Q10: making the loop self-feeding

What changes in `cycle.sh`:
- **It reads** `incidents.jsonl` as a second plan source after the file-based plan.
- **It may do, without a person,** everything marked "yes" in the Q3 table.
- **The one gate** is the card before `install-pii.sh`.

Add the 151 budget caps. Keep `PAUSED` meaning paused, which needs spec 153 first.

## Part 3: what not to build, and the first experiment

**Do not build, and the number that closes each:**

| do not build | the number |
|---|---|
| The `judge` node or a 27B typed-decision model now | 2 of 2 real incidents were found by an exact-string rule; 551 rows with 11 distinct values (C-run) |
| Replay-safe resume (143 R2/R3) | G8 decided against auto-resume; the measured crash leaves **no record** to resume; 0 runs are deployed |
| Autonomous quarantine or deploy | each changes production; the deploy has been exercised **0** times |
| A reader of Hermes or host logs inside pii | Hermes is not pii; `watchdog.sh` already owns gateway liveness (`26c3c89`) |
| Shrinking the codebase toward 15k lines | the proving-test loop never needs the whole harness in context |
| Anything that keeps 143/144 alive only because it was written | 143's four criteria and 144's three pass on a no-op; 144's fourth can never pass (C-run) |

**The first experiment, within a day, as a probe rather than a product:**
- **Detection half: already run here.** The fixed rule over the preserved log opened 3 incidents from 551 rows, 2 of them real problems a person fixed later. The benign one, `window-closed`, belongs on the benign list. Detection gained about 41 h.
- **Resolution half, still to run.** For the 3 `gate-failed` and 4 `verify-failed` historical cycles in `cycle-last-detached.log`, dispatch one budgeted worker each. Give it only the incident evidence, in a scratch export at that revision. Measure mechanically whether it leaves a named test that is red on base and green on its fix.
- **Decision, pre-registered:** build 151 if **at least 3 of 7** produce a mechanically proven fix at under $1 each. Otherwise stop at 150 plus 152, and let a person resolve with the evidence in hand.

## Part 4: NOT VERIFIED

| item | why | what would check it |
|---|---|---|
| The selftest's "23 assertions green" | running it writes the loop's real `notes.md`, `questions.md` and `state.json`, which the brief said not to disturb | spec 153, then run it |
| The gateway "dead for two days" incident | taken from commit `26c3c89`'s message | Hermes gateway logs |
| The resolution half of the experiment | needs paid worker dispatches | the probe above |
| Whether ids 145-148 are taken | my listing was obscured by a shell alias; inferred from the commit title | `command ls specs \| grep '^14[5-8]'` |
| Local per-row latency for any judge model | not measured; 144 is deferred anyway | time one `llamacpp-server` call on :8082 |
| The 060 write-up's figures and the video timeline | the brief said not to re-research | n/a |
| The brief's size numbers | I measured 24 workspace members (23 pii crates plus xtask), 400 tracked `.rs` files, 90,495 lines and 1,126 test attributes, against the brief's 25, 408, ~91,024 and 1,103 | `sed -n '/^members/,/^\]/p' Cargo.toml \| grep -c '"'`, `git ls-files '*.rs' \| wc -l`, `git ls-files '*.rs' \| xargs cat \| wc -l` |