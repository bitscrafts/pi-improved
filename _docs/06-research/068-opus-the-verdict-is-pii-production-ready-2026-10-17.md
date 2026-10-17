# 068: The verdict. Is pii production-ready? (Opus, 2026-10-17)

This is a read-only review. The only repo file written is this one, and nothing was committed. No scratch was
created, and I took no action on the running daemon (no signal, no restart). Evidence labels:
- **C-run**: I ran it today, 17:55-18:15 UTC.
- **C-read**: I read the lines.
- **asserted**: from a document, not re-checked.

Read first: 064, 065, 066, 067, `production-progression.md`, `INTERVENTION-CLOSED-2026-10-17.md`,
`INTERVENTION-2026-10-17-review-for-the-morning.md`, `scripts/autonomy/plan.tsv`.

_Written incrementally; each section stands on its own if the run is cut._

## 1. The verdict

**No. pii is deployed, identifiable and alive. It is not production-ready, because nothing it is configured to do
in production has ever run.** Measured just now on the deployed daemon:

| check | value | label |
|---|---|---|
| revision | `0e34719dce0d`, no `-dirty`: the card's revision | C-run (`pii health --json`) |
| uptime | 14.25 h, pid 870350, started 03:46:38 | C-run |
| argv | exactly the unit's `ExecStart` (`--store /root/.local/state/pii/store.redb --workspace /root/.local/state/pii …`) | C-run (`/proc/<pid>/cmdline`) |
| **runs** | **0**. The main store holds 0 `run` entries, 0 journal events, 0 graph bodies. | C-run |
| **the 09:00 UTC digest** | **did not run.** It is the only `[[cron]]` entry (`cron_entries: 1`). The daemon has been up since 03:46, and nothing exists in the store or the log. | C-run |
| channel | `errors: 57,289`, but that counter is cumulative in `channel.redb` across restarts. The current log shows 2 transient `EAGAIN` poll failures since 03:46 and **no** byte-budget refusal, so `027895e`'s fix holds. `cursor: 0`. | C-run |
| posts | `channel_posts: 1`: the start-up "is up" post. Nothing since. | C-run |

**Why nothing runs (C-run, C-read):**
- The daemon loads graphs from `runtime_dir.join("graphs")` (`crates/pii-cli/src/cmd_daemon.rs:391`), which is
  `/run/user/0/pii/graphs`.
- **That path does not exist.** `ls /run/user/0/pii/` shows `channel.redb channels control.sock learn memory
  timer.redb` and nothing else.
- The graphs are installed in `/root/.local/state/pii/graphs/`, and the unit's
  `ExecStartPre=/usr/bin/env ln -sfn /root/.local/state/pii/graphs /run/user/0/pii/graphs` is what should link
  them.
- So `start_run("discord-digest")` fails, and `Daemon::start_cron_entries` discards the error with `if let Ok`
  (`crates/pii-server/src/daemon/mod.rs:225`).
- The channel router does the same for `discord-triage` (`router.rs:120`, `let Ok(run_id) = … else { continue }`).
- **The failure is total and silent**, and every health subsystem reads `ok`.

**The strongest case against my own verdict, and why it does not survive.** The case: "production means a
supervised, gated, identifiable process that reports its own health and survives a crash. All of that is true
today, and graphs are configuration." It fails on the operator's own plan. M5's gate is
"`pii_channel_posts_total ≥ 7` over 7 days" (`production-progression.md`, M5). That can only be met by the digest,
and the digest cannot run. A process whose every scheduled and inbound behaviour is refused at the first step is
up, not in production.

**The strongest case against "production-ready" that survives my reading (the one I hold):** this is the **third**
consecutive deploy in which the measured product behaviour was zero while every instrument read green:
- 063: the triage ingress on a forum, `runs: 0`;
- 065: the channel dead after 3 h 29 min, with 43,918 errors and `incidents=0`;
- today: no graph loadable, and `runs: 0` again.

Each time the project's own verdict document called it live. The defect class is not any single bug. **A
production check is defined by what is installed and identifiable, never by what the product did.**

**Three corrections to the brief's "measured" list (C-run):**
1. **"14/14 PASS"** is **14 PASS + 1 SKIPPED** of 15 steps. `gate-run3-20261017T020154Z.log` shows
   `SKIPPED: coverage-floor (QG_SKIP_COVERAGE=1 — the floor was NOT measured in this run`. The coverage floor has
   not been measured on the deployed revision.
2. **"kill -9 returns it under systemd in 8 s."** The daemon's own log for the 03:46 restarts reads
   `daemon.stopped {reason: signal}` at 03:46:00 and again at 03:46:30, then `daemon.started` at 03:46:38. A process
   killed with SIGKILL cannot write `daemon.stopped` (`105-runbook.md:291`: "`daemon.started` with no
   `daemon.stopped` means the daemon died hard"). So the 8 s restart is measured, but it followed a **catchable**
   signal.
   - PID 1 in this container is `podman-init`, there is no systemd here, and the daemon's ppid is 0. Its
     supervisor lives outside this namespace, and I cannot see it.
   - The deploy log printed `systemctl --user enable --now pii` under "OPERATOR COMMANDS (nothing below was run)"
     (`deploy-0e34719-20261017T034126Z.log`).
   - The unit's `ExecStartPre` symlink is absent, and the process's cwd is `/projects`, not the unit's
     `WorkingDirectory=/root/.local/state/pii`. **Both are inconsistent with the process having been started by
     this unit.**
3. **"pii-mcp"** is **not swapped**: `pii-mcp` is still the 10-15 19:05 binary, and `pii-mcp.new` (10-17 03:45)
   sits beside it. The deploy log says "staged … NOT swapped". The one surface with daily real use runs a binary
   older than the deployed revision.

## 2. Against the original plan (`production-progression.md`)

| # | criterion, verbatim in substance | status | evidence |
|---|---|---|---|
| M0 | `selftest.sh` ALL PASS; `window.sh` honours the reset; a cut worker keeps its resume handle | **NOT MET** | the latest selftest verdict on record: "31 passed / 3 failed … `FAILURES ABOVE`" (skill-lane output `2026-10-17_12-22-20.md`). The loop also gated item 155 eight times today and was red at step one for 7 h 46 min on a hand commit (067 §1). |
| M1 | replay → exactly 1 `dirty-tree`, 1 `verify-failed`, 0 benign; **live**, one incident line reaches the phone within a day | **MET-WITH-CAVEAT** | the replay half closed by the loop (`e83ed6c`, asserted green). The live half is wired (`tick.sh:54` sends a line per new loop incident), but I did not verify that a line reached the phone. |
| M2 | `kill -9` 2 s into a run → `pii run get` answers `failed` "interrupted by daemon restart" + 1 reconcile event | **MET-WITH-CAVEAT** | met in its test (154, `c509d1b`). Never exercised in production, because production has had 0 runs. |
| M3 | proven fix on a branch; **pre-registered**: build 151 only if ≥ 3 of 7 historical cycles yield a proven fix < $1 | **NOT MET** | 151 was built (`ba0b8d5`) without the pre-registered experiment's result on record (063 Q4). On real incidents it proved **0 of 6** (all "no proving test recorded"). |
| M4 | the card: 5 fields; removing any one refuses it (5 reds); `DEPLOY_DRY=1` → yes prints the rev and the rollback | **MET-WITH-CAVEAT** | the tests are green (A21, "verified 4/4 by hand", 11 attempts). The first real deploy through it is 0e34719 (asserted in both INTERVENTION files; the deploy log itself names no card). |
| M5a | `pii health` identity == the deployed rev | **MET** | `0e34719dce0d`, no `-dirty` (C-run) |
| M5b | the unit survives 24 h on the host | **NOT MET (yet)** | up 14.25 h. The unit cannot be shown to be the supervisor from here (§1 correction 2). |
| M5c | `pii_channel_posts_total ≥ 7` over 7 days | **NOT MET, and cannot be met as configured** | `channel_posts: 1`, `runs: 0`. The digest's graph is not loadable (§1). |
| M5d | Hermes' call and error rates unchanged (±50% of 161/30 d) after the swap | **NOT MET** (untestable) | `pii-mcp` not swapped (§1 correction 3) |
| M6 | the same incident rule over the deployed daemon's events; a real incident from production within a week | **MET-WITH-CAVEAT** | `pii incidents scan --source runtime` on the live log: `incidents=1` (`refusal-repeated:channel.poll.failed`, count 57,287, latency 3 s) (C-run). But nothing schedules it (`grep -rn 'source runtime' scripts/ /root/.hermes/scripts/` matches only `plan.tsv` rows), it re-reports an already-fixed refusal, and it is **blind to today's total failure**, which emits no event. |
| M7 | retire the dormant surface: six decisions | **MET-WITH-CAVEAT** | 155 landed (`c53c813`); `m7-retiring-the-dormant-surface.md` records the decisions. The triage ingress on a forum (spec 165) is still bound. |

**Tally: 1 MET, 5 MET-WITH-CAVEAT, 5 NOT MET.**
- MET: M5a.
- MET-WITH-CAVEAT: M1, M2, M4, M6, M7.
- NOT MET: M0, M3, M5b, M5c, M5d. M5 is the milestone the plan calls "Production", and 3 of its 4 criteria are
  not met.

## 3. The smallest set of remaining work before the verdict could flip

None of the six queued rows is on the critical path. **The verdict flips on three things, and only one of them
is code:**

1. **The graphs load: config, plus a one-line hardening.**
   - Create the link the unit was meant to create, or start the daemon with the unit:
     `systemctl --user enable --now pii` on the host that owns the supervisor. This is a host act, so it needs the
     operator.
   - Then `discord-digest` runs at the next 09:00 UTC and posts. **Proof:** the store's `run` entries ≥ 1 by
     09:01 UTC, `channel_posts` ≥ 2, and a forum post naming the digest.
2. **A refused cron or ingress start is no longer silent (new row, code).**
   - `start_cron_entries` and `poll_once` emit `cron.start.refused` / `channel.run.refused` with the error, instead
     of `if let Ok` / `else { continue }`.
   - 156's refusal rule then opens an incident on the second refusal.
   - **Proof:** a daemon started with an empty graph dir prints the event within 1 s, and the runtime scan opens
     `refusal-repeated:cron.start.refused`. HEAD prints nothing.
3. **Seven days of the M5 soak, counted by the product.** `channel_posts` ≥ 7, no start without its
   `daemon.started`, the runtime scan scheduled in `tick.sh` against the deployed log, and the `pii-mcp` swap with
   Hermes's error share measured before and after.

Of the six rows: **A40/171 (two-tier gate)** and **A41/174 (warm cache)** decide whether the loop can keep the
product green, so they are next in value. **A39/169**, **A37/172**, **A38/173** and **A36/170** improve the loop
and do not change the product verdict. **168** (the deadline barrier) is done.

## 4. What would falsify this verdict

Run these, in this order, and any one of them proves me wrong:
1. `ls -la /run/user/0/pii/graphs` **on the namespace the daemon actually runs in** (the host, if the supervisor is
   there). If the link exists there and the daemon resolves graphs through it, my "not loadable" is wrong for that
   process.
2. `PII_RUNTIME_DIR=/run/user/0/pii pii health --json | jq .counts.runs`. **A value > 0** means a graph ran, and
   the core of my verdict is wrong.
3. Tomorrow at 09:01 UTC: a digest post in the forum and `channel_posts` ≥ 2. If both appear **with no change
   made**, then the digest was never broken, and I misread `cmd_daemon.rs:391`.
4. `kill -SIGKILL <pid>` on the host, then `grep -c daemon.stopped` over the new log tail. **0** stopped events and
   a new `daemon.started` within 10 s confirm SIGKILL supervision, and correction 2 is withdrawn. This is the
   operator's act; I did not signal production.

## 5. The honest failure mode of this project

**It certifies by proxy.** It measures what is installed, identified, gated or green, and reports that as what
the product does. Today's record shows it at every level:

| level | the proxy that was reported | the thing it stood for | what was actually true |
|---|---|---|---|
| product | revision, health subsystems `ok`, 1 post, "channel live" | the product does its job | 0 runs: no graph is loadable, and the error is discarded at `mod.rs:225` and `router.rs:120` |
| supervision | "kill -9 returns it under systemd in 8 s" | SIGKILL survival under the installed unit | a catchable signal (`daemon.stopped {reason: signal}`), and nothing visible here proves the unit is the supervisor |
| gate | "14/14 PASS" | the tree is release-grade | 14 PASS + 1 SKIPPED (coverage); and since the 14:29 split, 0 workspace test passes on 11 commits |
| loop | "converged, 33 closed, nothing open" (the first version of INTERVENTION-CLOSED) | the queue is done | corrected in place to "NOT CLOSED"; 6 rows open; 166 had closed on a criterion that could not fail |
| caches | "warm" (the morning doc's own admission) | no cold compile | a cache that no step read; then a shared cache that manufactured `NotFound` failures (067 §1) |
| diagnosis | "disk" | the cause | parallel linking, misdiagnosed twice on this machine (2026-08-11, 2026-10-17) |
| provenance | the citation corrected | a stale anchor fixed | the fix landed in the wrong artefact and invalidated the record (asserted, brief) |

**What it is most likely to get wrong next, specifically:** declaring M5 met after the operator fixes the graph
link, **on the first digest post**. One post proves the path once. M5's criterion is seven posts in seven days,
against a daemon that has restarted 8 times in 3 days (`starts_considered=8`). The next proxy will be "it posted",
standing in for "it posts".

**The rule that would stop it**, which the project already knows how to write:
- **Every production claim names a product counter and its threshold:** `runs ≥ N`, `channel_posts ≥ 7 over 7
  days`, `run.start.refused = 0`.
- **It is read from the running daemon, never from the deploy, the gate or the verdict document.**
- A claim with no counter is a claim about the machinery, and it is written as one.

## Final word

- **Verdict:** deployed, not production-ready.
- **The one blocker:** no graph can run (`/run/user/0/pii/graphs` is absent, and refusals are silent).
- **The one code change:** make cron and ingress start refusals emit events.
- **The one operator act:** start the daemon through its unit, or create the link.
- **The proof:** seven days of `channel_posts` from the product's own counter.

Nothing in the repo changed except this file, and nothing was committed. The uncommitted edits to `gate-steps.txt`,
`quality-gate.sh` and `deploy-runbook.md` in `git status` belong to the loop's in-flight item (A36/170).
