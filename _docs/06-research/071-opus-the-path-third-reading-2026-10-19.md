# 071: The path, third reading. The prediction, the occurrence, the order (Opus, 2026-10-19)

This is a read-only review: the only repo file written is this one, and nothing was committed. No signal was sent
to production. I used only `pii health`, `pii run ls` and the log files. Evidence labels:
- **C-run**: I ran it today, from 10:24 UTC.
- **C-read**: I read the lines.
- **asserted**: from a document or commit.

The brief is the Round-6 brief again. 069 (10-17) and 070 (10-18) answer it, so this file is the delta. It
records:
1. what 070 predicted, and what happened;
2. what changed in production;
3. the answers to the orchestrator's own gaps page (`_docs/05-operations/071-path-to-production-gaps-2026-10-19.md`,
   untracked), including its direct question about **order**;
4. the 12:00Z occurrence reading, appended when the clock reaches it.

_Written incrementally._

## 0. 070's prediction, scored

**070 §2, verbatim:** "tomorrow's occurrence executes **4** digest runs, one per armed copy".

**Result: 4 posts.** `channel.posted` at **09:00:01, 09:00:02, 09:00:02, 09:00:02 UTC** (C-run, `pii.log`).

The 4 runs that posted were **all cancelled beforehand** (C-run, `pii.log`):

```
10-18 22:01:55 run.cancelled r000000004161 "stale duplicate of the digest cron, retired after …"
10-18 22:01:55 run.cancelled r000000004193
10-18 22:01:55 run.cancelled r000000004225
10-19 07:42:04 run.cancelled r000000004129 "stale duplicate"
10-19 09:00:01-02  channel.posted × 4
```

So the prediction held, by a route 070 did not foresee. **`pii run cancel` closes the run record and leaves its
timer-wheel entry armed** (`c45b1ae`'s finding; spec 196, `5a6445e`, in the tree and not deployed). Between those
lines, every daemon start logged `cron.rearmed r000000004129` (03:44, 07:06, 07:07, 07:13). That run was cancelled
only at 07:42, and no `cron.rearmed` has been logged since. It still fired at 09:00 with the other three.

**Two corrections to the orchestrator's 071 page:**
- Its gap 1 says the cancelled runs "posted at **04:20Z**". The log says **09:00:01-09:00:02Z**.
- Its step-4 PASS (in `EXERCISE-RESULTS-2026-10-19.md`) is a `fresh-gate: PASS` on **`9045484`**, while production
  runs **`121fb6d`**. A gate on HEAD is not a gate on the deployed revision. **Step 4 is RED for the running
  binary**: no full gate is recorded on `121fb6d`.

## 1. Production now (C-run, 10:30 UTC)

| reading | value |
|---|---|
| revision, uptime | `121fb6de67e1`, 2.74 h (started 07:40:00) |
| `since_deploy` (new: 070 §5's proposal, `188`/`190`/`192`) | `armed 1, hard_deaths 0, starts 3, runs 5 (succeeded 4, suspended 1), posted 0, post_refused 0, unattributed 1,850` |
| `channel_posts` (process counter) | **5**: 1 start-up post plus the 4 digests |
| store | 1,866 runs: 1,836 failed (10-18), 29 succeeded, 1 suspended |
| schedule | graph timer `cron = "0 12 * * *"`, config `schedule = "0 12 * * *"` (moved from 09:00Z in `498e7ab`) |
| model | `openai:glm-5.3-flash`, "TEMPORARY 2026-10-18: ollama-cloud" |
| wired graphs | `[[cron]] digest → discord-digest`, `[[channel]] discord → discord-triage`. A third graph, `intervention-report` (`worker`, `gate`, `timer`, `post`, `memory`), sits in the graph dir, **unwired**. |

**A new contradiction in the new instrument.** `since_deploy.posted` reads **0**, while the 4 digest runs that
posted at 09:00 are counted in `since_deploy.runs_by_status.succeeded: 4`, and `channel_posts` reads 5. The block
built to attribute posts to the running deploy fails to attribute the 4 posts this deploy made. `81bcf90` names it
("the since_deploy blind spot for unattributed posts"). It has a name and no fix.

## 2. The orchestrator's question: is there a gap it has not seen, and would I change the order?

**Order: read the occurrence first, unconditionally.**
- It costs nothing and cannot be substituted.
- **The stream defect is not on the configured production path.** Neither wired graph calls a model:
  `discord-digest` is `timer, recall, transform, data, post`, and `discord-triage` is `classify, transform, post`
  (C-run, `grep '^kind'` over the installed graphs).
- The ollama-cloud truncation therefore affects the unwired `intervention-report` graph (`worker`), interactive
  `pii -p`, and nothing scheduled. It deserves its instrument, as the orchestrator says, but after the reading,
  not before it.

**Revised order:**
1. **12:01Z and 12:05Z readings, recorded verbatim** (§4). Unchanged from the orchestrator's item 1.
2. **Deploy 196 (cancellation disarms), then prove it ON A SCRATCH DAEMON with a two-minute cron**, then on
   production, with exactly one counted start. Moved up: until it ships, the operator's only stop control
   (`pii run cancel`) does not stop anything (§0).
3. **Fix `since_deploy.posted`** (0 against 4, §1). It is the instrument the soak (step 6) will be read from. A
   soak counted by a block that misses this deploy's own posts certifies nothing.
4. **The card is enforced in the installer** (orchestrator item 4). Agreed, and code is the right place.
5. **Instrument the stream defect** in the HTTP head loop and `chat_streaming` (orchestrator item 3), before
   `intervention-report` is wired to anything.
6. **Re-mark the exercise matrix line by line** (orchestrator item 5).

**Gaps the page does not list:**
- **G-a. The gate and the deploy name different revisions.** The fresh-gate PASS is on `9045484`, and production
  is `121fb6d` (§0). Nothing ties "gated" to "installed". The card is the place to bind them: the card's `rev` must
  equal both the verdict's `rev` and the build's stamp.
- **G-b. A cancelled run's wheel entry survives in the deployed binary**, and nothing asserts which run a start
  re-arms. `cron.rearmed r000000004129` was logged at 4 starts, one of them after its three siblings had been
  cancelled. 196 addresses the wheel. A test that a start re-arms **only** the entry's current, uncancelled run
  does not exist yet.
- **G-c. `since_deploy` cannot see posts** (§1). The orchestrator calls this "named". It is also on the critical
  path, because step 6 depends on it.
- **G-d. A graph sits in production's graph directory unwired**, with a `worker` node and a model that is
  "TEMPORARY". The daemon loads every graph in the directory, so one config line wires it. Nothing records that it
  is intended to stay dormant (M7's rule for the dormant surface).

## 3. The five questions, as of now (the deltas from 070)

**1. The path.**
- 070's seven steps stand. Step 1 (one armed run) is green by `since_deploy.armed 1`.
- Step 2 is decided at 12:00Z today.
- Steps 3-5 have changed state:
  - **step 3:** 8 starts since 10-18 12:00 against 2 installs, improved;
  - **step 4:** **red for the running revision** (G-a);
  - **step 5:** 3 cards against 11 asides (asserted, `EXERCISE-RESULTS`).
- **Insert before step 6:** "`since_deploy.posted` counts this deploy's posts". The soak is read from it.

**2. The redeploy (of 196).**
- **Before:** the card names a rev whose fresh-gate PASS line names the same rev. On a scratch daemon with
  `*/2 * * * *`: cancel the armed run, and **0** executions follow at the next occurrence. On HEAD today that is 1,
  per `c45b1ae`.
- **After:** `since_deploy.revision` == the card's rev. `cron.rearmed` names a run id **not** in the cancelled
  set. `since_deploy.armed == cron_entries == 1`.
- **Then:** at the next 12:00Z, +1 run, +1 post (counted by **both** `channel_posts` and `since_deploy.posted`),
  0 refused, and the same at 12:05Z.

**3. 068 now.**
- **(a) Fixed in the tree:** cancellation disarms (196), refused-post pacing (195), poll pacing (194), deploy
  attribution (`since_deploy`).
- **(b) Still true in production:**
  - the deployed binary lacks 196;
  - `since_deploy.posted` misses this deploy's posts;
  - no gate on the deployed revision;
  - `pii-mcp` not swapped;
  - the digest content is "0 device(s), 0 lesson(s)".
- **(c) Newly true:**
  - the digest now posts at 12:00Z;
  - production's model is a temporary cloud provider with an unresolved truncation defect, on no wired graph;
  - an unwired worker graph sits in the graph directory;
  - **4 posts at 09:00Z from 4 runs the operator had cancelled.**

**4. The remaining risk (three sentences).**

> pii on the host (`121fb6d`) posts its daily digest at 12:00 UTC, and this morning it posted four times at 09:00 UTC
> from four runs the operator had cancelled, because the deployed binary does not disarm a cancelled run's timer
> (fixed in the tree as 196, not deployed). Until 196 is deployed, cancelling a run is not a way to stop it, so
> disable the `[[cron]]` block instead. Treat no reading as production evidence unless `since_deploy.posted` agrees with
> `channel_posts`, because today it reads 0 against 4.

**5. What it still cannot see.**
- **It cannot see that its own "done" applies to a different revision than the one running.**
  - The gate verdict is `9045484`, which contains 196. The deploy is `121fb6d`, which does not.
  - The orchestrator scored step 4 PASS on a revision that is not installed. That is the same class as 068's
    "certifies by proxy", one level up: a correct measurement attached to the wrong object.
- **The instrument:** one record that binds `{card.rev, gate.rev, build.stamp, since_deploy.revision}`. Every
  production claim is refused unless all four are equal.
- **Proof:** today it reads `{none, 9045484, 121fb6d, 121fb6d}`, which is refused, and that refusal is the true
  state.

## 4. The 12:00Z occurrence (070 step 2): the reading

Recorded verbatim by the background reader (C-run, `<scratch>/rev071-occurrence.txt`):

| reading | runs | `channel_posts` | refusals | `since_deploy` | `run ls` |
|---|---|---|---|---|---|
| 10:25:13 baseline | 1,866 | 5 | 0 | armed 1, runs 5, succeeded 4, suspended 1, **posted 0** | suspended 1, succeeded 29, failed 1,836 |
| **12:01:15** | **1,867 (+1)** | **6 (+1)** | **0** | armed 1, runs 6, succeeded 5, suspended 1, **posted 1** | suspended 1, succeeded 30, failed 1,836 |
| **12:05:15** | 1,867 (unchanged) | 6 (unchanged) | 0 | unchanged | unchanged |

Log, from 11:59Z: exactly one event, `12:00:02 channel.posted {bytes: 94, channel: discord, destination: 1558589728671146105}`.

**Verdict on 070 step 2: PASS.** It is the first scheduled occurrence in this project's history to produce
exactly one run and one post, with the chain re-armed to one. It held on the deployed `121fb6d`, without 196,
because no cancelled duplicate remained in the wheel for the 12:00 slot.

What it does and does not settle:
- **It settles** that the deployed scheduler, with one armed run, fires once per occurrence and re-arms once. That
  matches 070 §0.2's scratch probe, now in production.
- **`since_deploy.posted` counted this post (0 → 1)**, so the instrument works for runs armed by this deploy. It
  still misses the 4 posts at 09:00Z, which came from runs armed by an earlier process. The blind spot narrows from
  "cannot see posts" to "cannot attribute posts from runs it did not arm". Step 6's soak can be read from it **only
  if no restart-inherited run fires during the soak**.
- **It does not settle** cancellation (196 is not deployed), the restart path (one start re-arming only the current
  run, G-b), or anything about seven days.

**The 068 verdict after this reading:** still not production-ready. But for the first time, the reason is time and
known, named deltas (196 deployed, the soak, the card and gate bound to the running revision), and no longer an
unexplained behaviour of the product. **Day 1 of 7 of the soak can be counted from 12:00Z today, if and only if the
deploy that carries 196 does not reset it.** If it does reset it, the soak's first day starts at that deploy's
first occurrence.
