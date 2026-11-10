# 063: The next phases on the path to production (Opus, 2026-10-15)

This is a read-only review. The only file written is this one, and nothing was committed. Evidence labels:
- **C-run**: I ran it today.
- **C-read**: I read the lines.
- **asserted**: from the brief, not re-checked.

Inputs read first:
- `production-progression.md` (M0..M7)
- 059 (§0, §1.1-1.4, §2.1, §2.3-2.6, §3, §4, §5)
- 061 (in full)
- 062 (in full)

_Status: written incrementally. Sections are appended as they are measured._

## Verdict

pii is **running, but it is not yet in production.** The daemon is up at revision `a5e51fb13cd6-dirty` and has posted
once. But four things are true:

- It is a child process of an agent session, not a supervised service.
- Its log does not survive its own restarts.
- The incident reader built for it is blind on the real log (0 of 5 starts seen) and false on the audit log (5 of 5).
- The only thing it will post tomorrow is "0 device(s), 0 lesson(s)".

Meanwhile, the one pii surface with real, growing use is `pii-mcp`: 137 Hermes calls today, still the 2026-09-09
binary. And no deploy has gone through the deploy card.

The next phases are therefore not new capability. In order, they are:
1. make the running thing a production process;
2. make the reader read the real record;
3. make each phone line carry information;
4. freeze what has no input;
5. then put it in front of a second person.

## 0. State, re-measured (not taken from the brief)

| brief says | measured today | label |
|---|---|---|
| M5 done: release installed, daemon RUNNING `a5e51fb13cd6` | Installed binaries `pii`, `pii-web`, `pii-evolve` are dated 2026-10-15 19:05. The daemon is pid 3369179, started 20:00:58. **Its parent is `hermes --yolo` (pid 520685, up since 10-06), not a unit.** PID 1 is `podman-init`, and `systemctl` says "System has not been booted with systemd". The installed `pii.service` therefore cannot run here, so M5's "the unit survives 24 h" cannot be measured as written. The argv is a bare `pii daemon`, not the unit's `--store $STATE_DIR/… --workspace $STATE_DIR`. As a result, the store lives in `/run/user/0/pii` and the rw sandbox root is **`<repo>`**, the tree the builder gates. | C-run (`ps`, `/proc/<pid>/{cmdline,cwd}`, `pii health --json`) |
| revision `a5e51fb13cd6` | `pii health` reports `a5e51fb13cd6-dirty`. Commit `a5e51fb` is a **"leftover … kept for review"** commit: an ungated diff to `cmd_daemon.rs`. Production runs a revision that never passed the gate, and it was built from a dirty tree, so its source cannot be recovered from git. | C-run, C-read |
| `channels: [{kind: discord}]`, `channel_posts: 1` | True. The one post is the start-up `channel.verification_posted` ("pii 0.1.0 is up"). `runs: 0` (store `run` entries 0 since 14:52), `memory_entities: 0`, `cursor: 0`, `errors: 0`. | C-run |
| (not stated) | **5 daemon starts today** (14:52, 17:02, 19:05, 19:06, 20:01 UTC, from `audit.log`). `pii.log` holds **3 lines**, all from the last start. The four earlier starts left nothing in the file 156's reader is meant to read (cause not determined; the writer opens in append mode, `filelog.rs:97`). | C-run |
| 156 CLOSED, 5/5 with a clean-log control | On a **copy** of today's real logs (scratch `<scratch>/rev063-6pbi`), `pii incidents scan --source runtime` returns **0 incidents from `pii.log`** (5 starts, none visible) and **5 `daemon-start-not-ready` from `audit.log`**, all false: every start reached `daemon.started`, but that event goes to `pii.log`, not `audit.log`. Nothing schedules this scan: `tick.sh:54` scans only the loop's own logs, with `target/debug/pii`. The state file says "verified **by hand**". | C-run |
| 157 CLOSED fa824b0, 3/3 | The implementation `fa824b0` has a hand-made message shape, followed by an automated pass `075fee9` (A24, 3 attempts). The configured `PII_DISCORD_CHANNEL_ID=1558589728671146105` is the **forum** (`_docs/02-guides/discord-surface.md:10`, "type FORUM"). The ingress (`discord.rs:101-112`) GETs `/channels/<forum>/messages`, and the poll has no filter for the bot's own posts (`discord.rs:150-170`). | C-read, C-run |
| 152 CLOSED 4/4 | `state.json` A21: attempts 11, "verified 4/4 **by hand**". **The real deploy did not go through a card**: `questions.md` and `operator-acts.log` have 0 card entries. | C-run |
| 155 `7debbdf` | It is committed, but `state.json` A22 still reads `verifying`, "plan gate running". | C-run |
| 150, 151, 154 | Automated passes: `e83ed6c`, `ba0b8d5`, `c509d1b`. **151 on real incidents: 6 of 6 parked**, each "no proving test recorded in the incident: nothing can be proven" (`_tmp/autonomy/incidents/*/state`). The pre-registered "≥ 3 of 7 historical cycles" decision has no recorded result: `_tmp/autonomy/experiment/` holds seed manifests only. | C-run |
| 158 committed, 159/160 drafted | 158's spec is at `specs/158-…`, and the loop is dispatching it now. **The stall watch fired at 20:30**: "worker 158-one-point-of-control … stream idle 1700s". I found no file for 159 or 160 under `specs/`, `_tmp/`, or anywhere on the root filesystem. | C-run |
| warm gate (062 rank 1) | `fresh_gate.rs:199` reads `FRESH_GATE_TARGET`, and `env.sh:24` sets it to `_tmp/gate-target`. **That directory is 4.0 K and has not been written since 12:29.** I have not proven that today's gate runs used it. | C-run |
| `cargo` shim behind the lock (062 rank 2) | Not present. `cargo` resolves to `/usr/local/bin/cargo`, a rustup-home wrapper. Leftovers are still committed: `cycle.sh:130-133`, 2 since 062. | C-run |
| (not stated) | **Hermes uses `pii-mcp` more, not less.** It made 137 `mcp__pii__*` calls today and 324 since 10-08 (322 `bash`), against 059's 161 per 30 days. The binary is still `pii-mcp` from 2026-09-09; `pii-mcp.new` (10-15 19:05) sits beside it, unswapped. | C-run, read-only `state.db` |
| (not stated) | Phone traffic from the pii Hermes jobs today (all `deliver=all`): loop 14 of 38 runs delivered, supervisor 6 of 6, skill lane 3 of 3, stall watch 2 of 50. Of the 25 deliveries, **8 name an outcome or a stall**: 4 "done", 2 "needs a hand", 2 STALL. 12 are "in flight N min" or "supervisor idle". On top of those, each daemon start is a post into a forum. | C-run (`~/.hermes/cron/output`) |

## Q3. What breaks first when the daemon runs unattended and its digests reach a phone, and which instrument shows it

These are ordered by when each one happens: the first is already measurable, the last needs a week.

1. **The evidence of a restart disappears before anyone reads it.** It has already happened four times today. Five
   starts left 3 lines in `pii.log`, so 156's reader sees none of them, and on `audit.log` it raises 5 false
   incidents (§0).
   - Unattended, a crash-and-restart either raises no alarm or raises one every time. Either way the operator stops
     trusting the line.
   - **Instrument that shows it:** `grep -c daemon.started pii.log` against `grep -c daemon.sandbox.in_force audit.log`.
     Today that is 1 against 5. The two must be equal.
2. **Nothing restarts the daemon, and nothing owns it.** It is a child of a Hermes agent session (`--yolo`, pid 520685).
   When that session or the container goes away, so does production. The installed unit cannot help: there is no
   systemd in this container.
   - **Instrument:** `ps -o ppid= -p $(pgrep -f '^~/.local/bin/pii daemon')` must name a supervisor, not
     `hermes`.
   - `pii health`'s `uptime_ms` is no help, because it resets with every start (1,570 s at 20:27).
3. **The first scheduled digest, at 09:00 UTC (06:00 local), says nothing.** `discord-digest` posts
   `0 device(s) known, 0 lesson(s) remembered`, because `memory_entities: 0`: nothing writes entities, which is 059 F9
   and still true.
   - Its `to` is the forum id, so each digest opens a **new forum thread**, and so does each "is up" post.
   - **Instrument:** the posted text itself, and `memory_entities` in `pii health`. A digest whose two numbers have been
     0 for 7 days is a defect, not a quiet week.
4. **The ingress is dead and looks healthy.** It polls `/channels/<forum>/messages` every 500 ms, and a forum's
   conversation lives in threads.
   - After 26 minutes it showed `cursor: 0`, `errors: 0`, `runs: 0`, which is indistinguishable from "nobody wrote".
   - The poll also has no own-author filter. If it is ever pointed at a thread or text channel, its own `[class] …`
     echo comes back as input. That is guarded only by the 20-message window and the cursor.
   - **Instrument:** none exists today. The smallest one is `cursor` unchanged and `runs` unchanged within 24 h of a
     **known** operator message in the thread.
5. **Attention runs out before any of the above is fixed.** 25 pii lines reached the phone today, and only 8 named an
   outcome. A phone that buzzes 25 times for 8 facts gets muted, after which the 41.5-hour gap that 150 closed comes
   back through the side door.
   - **Instrument:** the signal share, computed over `~/.hermes/cron/output/<pii jobs>/` as delivered lines that name
     an outcome, stall or incident, divided by all delivered lines. Today it is 8/25 = 32%.
6. **The M5 counters cannot be read across a restart.** `channel_posts` is per process: 5 starts happened, and it
   reads 1. So M5's "`pii_channel_posts_total ≥ 7` over 7 days" is unmeasurable as specified.
   - The Hermes criterion ("±50% of 161/30 d") is also confounded. Hermes's own use of `pii-mcp` has risen sixfold
     with no pii change at all.

**First to break: (1) and (2), both already measurable today.** (3) breaks tomorrow at 09:00 UTC. (5) is the one
that ends the experiment.

## Q4. What built today is premature and should be FROZEN rather than extended

| freeze | the number that says so | what "frozen" means |
|---|---|---|
| **151, the incident-to-branch dispatch** (second plan source, `cycle.sh:66-80`) | **0 of 6** real incidents proven, all 6 parked "no proving test recorded". The pre-registered ≥ 3/7 experiment has no result. The incidents 150 opens are loop states (`dirty-tree`, `locked`, `add-failed`, `no-change`, `parked`, `verify-failed`), not product defects with a test. Every park still costs a dispatch: the 20:12 cycle cost $0.06 to park `add-failed`. | Stop dispatching from `incidents.jsonl`, and keep the code. Reopen only when one incident carries a failing test name, which needs Q3(1) first. |
| **152, the deploy card** | 11 attempts, closed by hand, and **not used for the one real deploy** (0 card entries anywhere). A permission that the deploy bypassed is not yet the permission. | No new fields. The next deploy goes **through** it, or the card is redesigned around how the operator actually deployed. |
| **158, the mutating verbs** (`gate off`, `restart`, `stop-pid`) | The worker has been idle for 1700 s right now. The verbs live outside the repo (`<agent-host-config>`), outside the gate. Each is one more way to stop or start things, which is the class 158 exists to shrink. | Keep `ctl.sh status`, read-only, with its `UNOWNED` section: that is the actual repair. Freeze the verbs. |
| **160 ("pii heals itself")** and any new anomaly kind in 156 | The detector is blind or false on the real log (§0). Healing on a false incident is worse than no healing. | Do not draft 160 further until Q3(1) is green. |
| **159 as specified** (H_pre / L / A) | See Q2: `A` is undefined for 6 of 6 incidents, because none reaches a card. | Replace with the smaller measure below. |
| **The `discord-triage` ingress on the forum** | `runs: 0`, `cursor: 0`. It cannot see a thread, so it serves no one. | Remove the channel's `graph` binding, or point it at one thread (see Phase E). Do not extend `classify`. |

Keep, because they are earning: the stall watch (it fired correctly at 20:30), `status-line.sh`, `health.sh`, 150's
detection over the loop's records, and 154.

## Q2. Is H_pre / L / acts-per-incident the right next measurement?

**No. Not next.** Three reasons, all measured:

1. **H_pre is fed by `operator-acts.log`, which records the switch, not the work.** It has 9 lines in 2.5 days, while
   **66 hand-made commits** landed since 2026-10-13T15:00. H_pre would undercount the operator's work by about 7×,
   and would read "zero-touch" on a day spent repairing the loop.
2. **A (time to the card) is undefined for every incident that exists.** 6 of 6 park before a card. The primary
   measure, "share of incidents reaching a card with H_pre = 0", reads 0/0.
3. **The answer is already available, and it is negative.** Using the message shape from git, the instrument 061
   used:

   | window | all commits | loop closures ("automated pass") | leftovers | hand | hand per closure |
   |---|---|---|---|---|---|
   | 061 baseline, 10-11T15:00 → 10-13 | 44 | 10 | n/a | 34 | **3.4** |
   | since 10-11T15:00 | 120 | 16 | 10 | 94 | **5.9** |
   | since 10-13T15:00 | 82 | 9 | 7 | 66 | **7.3** |

   Since 10-13T15:00 the hand commits break down into 26 loop repairs, 22 records or docs, 9 spec authoring and 9
   other. Among the "other" are the hand implementations of 155, 156 and 157.
   **The loop is currently increasing the operator's work, not reducing it.**

**The smaller measure, which says sooner:** compute two numbers weekly from git and `state.json`, without a new spec:
- **R = hand commits per loop closure**, excluding spec authoring, which is the planner's job;
- **K = items closed "by hand"**, counted from `state.json` `last` fields.

Today K is 2 (152 and 156). The loop reduces the operator's work when R falls below the 3.4 baseline for **two
consecutive weeks** and K is 0. Detection latency L is worth keeping, but as 062's **time-to-notice** on induced
deaths (Phase B), not as a by-product of incidents that never reach a card.

## Q1. The next phases, from where we actually are

September's plan assumed the hard part was building the loop, and that production would follow. Measured today,
production exists but is unowned, and the loop's closures cost more hand work than they save. The phases below
change what runs, not what is built. Each criterion is red today, as stated.

### Phase A: production is a supervised process running a gated build, and the used surface is current

- A supervisor that exists in **this** container owns the daemon. That is the operator's decision; the options are
  a no-agent Hermes job, the host's systemd outside the container, or s6/tini.
- The argv is the unit's: `--store $STATE_DIR/store.redb --workspace $STATE_DIR --log-file …`. Production's write
  root must stop being the builder's repo.
- The installed revision is a commit that passed `fresh-gate` in its own cycle log: not `-dirty`, and not a
  "leftover" commit.
- Then swap `pii-mcp` (059 §2.4 trap 1), behind a `tools/list` probe. It is the most-used pii surface (137
  calls today) and still runs the 2026-09-09 build.
- **Criterion:**
  - `kill -9` the daemon, and it is back within 60 s with a new `daemon.started`;
  - its parent pid is the supervisor, not `hermes`;
  - `pii health` `revision` has no `-dirty`, and `git log -1 --format=%s <rev>` does not begin with "leftover";
  - `readlink /proc/<pid>/cwd` is not `<repo>`;
  - after the swap, Hermes's `mcp__pii__bash` error share over 7 days is no higher than the 7 days before.

### Phase B: the reader reads the real record, and a death is noticed in minutes

- `pii.log` keeps every start, and `daemon.started` and the start mark are written to the **same** file.
- `tick.sh` runs `pii incidents scan --source runtime --log <deployed pii.log>` with the **installed** binary, not
  `target/debug/pii`.
- This is 062's time-to-notice, applied to production instead of a scratch tree.
- **Criterion:**
  - On a copy of today's logs: **0** `daemon-start-not-ready` (today 5, all false).
  - Over 3 induced `kill -9`s of the supervised daemon: 3 `daemon-restart-unexpected`, each reaching the phone, with
    a **median ≤ 120 s** from the kill to the phone line. Today the result is ∞: nothing runs the scan.
  - `grep -c daemon.started pii.log` equals the number of starts.

### Phase C: every line on the phone earns its place

- The digest either carries content or is disabled. With `memory_entities: 0` it cannot carry content, so the
  honest options are:
  - a digest built from what the daemon **does** know: runs, refusals and incidents from Phase B;
  - turning the cron off.
- "is up" goes to one fixed thread, not a new forum thread per start.
- The loop's "in flight N min" and "supervisor idle" lines leave the phone. They belong in `status-line.sh`, which
  already exists.
- Posts are counted across restarts, read from `channel.redb` or the log, not from a per-process counter.
- **Criterion, over 7 days:**
  - the signal share of delivered pii lines is **≥ 80%** (today 32%);
  - the posts counter read after a restart is ≥ the value before it;
  - 0 digests posted whose every number is 0.

### Phase D: freeze, and measure operator distance with what exists

- Apply the freezes in Q4.
- Report R and K (Q2) weekly, from git and `state.json`.
- No new loop feature lands while R is above baseline. This is 062's "stop growing the loop", given a number that
  ends it.
- **Criterion:** R (hand commits per loop closure, spec authoring excluded) **≤ 3.4 for two consecutive weeks**, with
  K = 0. Today R is 57/9 ≈ 6.3, and K is 2.

### Phase E: a second person

See Q5.

## Q5. The smallest next phase that puts pii in front of a real user other than the operator

**There is no such user today.** Hermes is the operator's agent, not a second person. The forum has had one poster:
the bot.

The smallest real step is **one named person, one forum thread, one answer**:
- bind the ingress to **one thread id**, which is a text-shaped channel that `/messages` can read, instead of the
  forum id;
- add an own-author filter, by bot user id, so the echo cannot loop;
- bind the thread to a graph that **answers**: an `agent` node on `llamacpp-server:granite-4.2-3b`, read-only caps,
  and `post`. `classify` only echoes the class back.

This is configuration plus one small filter, not a new pillar. It depends on Phase A (a supervised daemon) and on
Q3(4) (an ingress that is not dead).

**Criterion:**
- one message from a Discord user id **other than the operator's and the bot's** produces exactly one reply in that
  thread within 60 s;
- the reply is recorded as a daemon run whose `input.user` is that person;
- over the following 7 days that person starts **≥ 3 runs unprompted**.

The unprompted count is the number that says "user", not "demo".

## Final ranking (at most five phases, by effect)

| rank | phase | why this rank | measured criterion (red today) |
|---|---|---|---|
| 1 | **A. Production is supervised, gated, not in the builder's tree; `pii-mcp` swapped** | Everything else measures a process that can vanish with an agent session, built from an ungated `-dirty` revision. The swap upgrades the one surface with real use (137 calls today on a 5-week-old binary). | `kill -9` → back ≤ 60 s under a non-Hermes parent; revision not `-dirty` and not a leftover commit; cwd ≠ repo; Hermes `mcp__pii__*` error share not up over 7 days |
| 2 | **B. The reader reads the real log; time-to-notice on production** | Today detection on production is blind (0 of 5 starts) or false (5 of 5). This is the step that makes unattended running safe. | 0 false `daemon-start-not-ready` on today's log copy; median kill-to-phone ≤ 120 s over 3 kills |
| 3 | **C. Every phone line earns its place** | Attention is the resource that ends the experiment: 8 of 25 lines carried a fact today, and tomorrow's digest carries none. | signal share ≥ 80% over 7 days; 0 all-zero digests; posts counter monotonic across a restart |
| 4 | **D. Freeze (151 dispatch, 158 verbs, 160, 159 as written, forum triage); measure R and K** | The loop costs 6.3 hand commits per closure against a 3.4 baseline. Growth must wait for a number that says it pays. | R ≤ 3.4 for two consecutive weeks and K = 0 |
| 5 | **E. One second person, one thread, one answer** | It is the first evidence that pii is used, not just running. It is cheap once A and C hold. | a non-operator user id gets one reply within 60 s, recorded as a run; ≥ 3 unprompted runs in 7 days |

## NOT VERIFIED

| item | why | what would check it |
|---|---|---|
| Why `pii.log` lost the four earlier starts | the writer appends (`filelog.rs:97`); I did not find who moved or truncated it | the start commands used at 14:52, 17:02 and 19:05 (Hermes session history) |
| What Discord returns for `GET /channels/<forum>/messages` | I made no outward call with the operator's token | one read-only GET, or a scripted `HttpTransport` test |
| Whether today's gate runs used `_tmp/gate-target` | the directory is empty, and today's cycle logs did not show gate compile lines | `du -sh` after the next gate, plus `Compiling` registry lines in its output |
| Whether every `deliver=all` line actually reached the phone | I counted Hermes output files, not delivery receipts; `last_delivery_error` is `None` for all four jobs | the gateway's delivery log |
| Classification of the 3 skill-lane deliveries | their output files embed the skill prompt, and I did not separate the reply | read the reply section of each |
| 159 and 160 contents | no file found | the operator's draft location |

Scratch used: `<scratch>/rev063-6pbi/` (copies of `pii.log` and `audit.log`). Nothing in the repo was changed
except this file. Nothing was committed.
