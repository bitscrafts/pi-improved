<!-- Filed 2026-11-02T05:34:29Z from an Opus plan-mode analysis run.
     Produced by: Claude Code + claude-opus-5-5, plan mode, READ-ONLY (no project writes).
     Repo HEAD at analysis time: f390f6a (tree: 0 changed paths)

     STATUS: NOT a spec and NOT a plan of record. Nothing in this document is authorised work.
     Every proposal needs its own pre-registered gate before implementation, and any idea previously
     closed by a measurement stays closed until a NEW measurement reopens it.

     Anything marked "asserted-from-memory" is NOT verified — treat it as a lead, not a fact. -->

# pii — is this production, and is this the path? An audit of the status as of 2026-11-02

Analysis only. Nothing in the repository was changed. Every number below was re-measured on 2026-11-02 between 05:20 and
05:45 UTC, either in the `hermes` container (`/projects/pi-improved`, HEAD `f390f6a`, 0 dirty paths) or on the host
`lnx` over `ssh mvcorrea@10.10.10.217` using read-only commands. Where I could not measure something, I say so.

## Contents

- **Part 1 — The status, audited**
  - 1.1 Verdict in one paragraph
  - 1.2 The milestone table, re-scored (Q1)
  - 1.3 Premises in the brief that are wrong
  - 1.4 The riskiest assumption (Q5)
  - 1.5 The loop's autonomy, measured (Q3)
- **Part 2 — The path, repaired**
  - 2.1 What production needs that no milestone asks for (Q2)
  - 2.2 Where the leverage is (Q4)
  - 2.3 What to cut (Q6)
  - 2.4 The next seven days (Q7)
- **Part 3 — The operator's decision**
- **The single next action**

---

# Part 1 — The status, audited

## 1.1 Verdict in one paragraph

What runs on `lnx` is **a deployed daemon, not production.** Its identity is proven, and so is one on-demand post.
Today I also measured five defects on the host that no gate sees:

1. Every run's trajectory write fails, because production's config points at the container's `/root` paths.
2. The two stores that promise "a crash cannot duplicate or lose a message" sit on a tmpfs.
3. Plain `pii health` on the host answers "no daemon answered".
4. The installer that produced production skips the deploy card, and it copies the dev stores over production's.
5. The M6 detector already finds a real production incident in today's log, but no process runs it, and on the same
   log it raises 8 false start alarms out of 8 starts.

Separately, **M3 was built in breach of its own pre-registration**. The 7-cycle experiment *was* run (2026-10-13/14,
$3.04, 9 rows), produced 0 proven fixes, was declared void because of a harness bug, and that bug was never fixed.
All 15 gates are green and all 42 selftest rows pass. Both run in the container. Production runs on the host. The
distance between the two is the finding.

## 1.2 The milestone table, re-scored (Q1)

Verdicts: **evidenced** means measured on the thing it claims about. **mechanism-only** means proven on a fixture or
in the container, not on production. **stale** means the evidence is about something that is no longer the case.
**breached** means the milestone's own stop rule fired and was not honoured.

| | milestone | claim in brief | my verdict | the evidence I accept | the evidence that is a mechanism claim | the one command that moves it |
|---|---|---|---|---|---|---|
| M0 | builder loop left alone | done | **mechanism-only (demote)** | `selftest.sh` 42 rows; the beat is alive (`watchdog.sh` pid 1564253) | "left alone": `_tmp/autonomy/operator-acts.log` has **85 pauses and 75 resumes between 10-23 and 10-30**. **84 of the 85 pauses are tagged "orchestrator"**, i.e. an interactive session, logged as `human`. The loop has made **no commit since 2026-10-30**. The original tick job `37ff4aecb8b0` has been **disabled since 2026-10-19**; work moves through `pii-drain.sh` every 5 min plus a skill lane. See §1.5. | `git log --since=<7 days ago> --grep='automated pass' --oneline \| wc -l` against the total commit count. "Left alone" means the loop's share is the majority, not 0 of 62 (the count since 10-31). |
| M1 | detection | mechanism done, delivery wired, live half unexercised | **mechanism-only, and the delivery check can pass while broken** | replay is evidenced (spec 150 closure); today's tick log prints `incidents: up to date (0 line(s) published …)` on every beat | `_tmp/autonomy/INCIDENTS.md` **does not exist**, and neither does its cursor. `~/.hermes/scripts/pii-incidents-discord.sh:9` is `[ -f "$MD" ] \|\| exit 0`, so "the record was never written" and "no incident" both produce the same silence. That is exactly the check that passes while the live path is broken. It also reads **builder** logs only, never production's. | Run `incidents-md.sh` against a fixture log with one incident in a scratch `AUTONOMY_LOGD`/`PII_INCIDENTS_MD`, then point the Discord script at that file. It must print exactly one line. Then make the missing-file case loud. |
| M2 | crash evidence | done | **evidenced for a process crash; unevidenced for a host reboot** | gate row `daemon-crash-leaves-evidence.sh`; the run record lives in `store.redb` on disk | on production, `channel.redb` and `timer.redb` live in `/run/user/1000/pii`, a **tmpfs** (`pii health` → `stores: channel: /run/user/1000/pii/channel.redb`, `timer: /run/user/1000/pii/timer.redb`; `mount` → `tmpfs on /run/user/1000`). Placement is `crates/pii-server/src/daemon/mod.rs:201,204`. A host reboot loses the cursor and the timer wheel. The host has been up since **2026-09-16**, so this has never been exercised. | `ssh lnx 'PII_RUNTIME_DIR=/run/user/1000/pii ~/.local/bin/pii health \| grep -E "channel:\|timer:"'`. It is fixed only when neither path starts with `/run/`. |
| M3 | resolution on a branch | loop proven; economics and real worker not | **breached** | `incident-to-branch.sh` legs (base_rc=1, branch_rc=0, master unchanged) with a **scripted** worker | The pre-registered experiment **was run**: `_tmp/autonomy/experiment/results.tsv` (2026-10-14) has 9 rows from real `claude -p` workers (`worker-13{2..7,9}.json`), $0.22–$0.53 each, **$3.04 total**, and **0 proven**. Every row reads `base rc=0, fix rc=0`. `_docs/05-operations/experiment-harness-open-work.md:29-31` explains why: the base run does not contain the worker's test, so cargo runs 0 tests and exits 0. "All 9 rows in `results.tsv` are void." The fix is described there (lines 33-43) but never landed: the last commit to `scripts/experiments/` is `e4d5b75`, 2026-10-13. The rule in `production-progression.md` was "build 151 only if ≥3 of 7". 151 was built and is listed as "DONE (pre-registered condition)" (`PATH-TO-PRODUCTION-2026-11-01.md:15`). | Fix harness defect #6 with its mandatory control ($0 model spend), then re-run the 7 workers (≈$3, operator-approved). That produces the verdict the project pre-registered. |
| M4 | deploy card | done | **stale: bypassed by the installer that made production** | `deploy/install-pii.sh:151` refuses without a card | production was installed by `deploy/install-pii-user.sh` (commit `b6e4f4e`, 2026-11-02 02:26). That script contains **no card check** (`grep -c card` → 0). Lines 52-54 copy the container's `store.redb memory.redb approval.redb heal.redb` **over** production's. Re-running it silently replaces production data with dev data. | `grep -nE 'card\|CARD' deploy/install-pii-user.sh` must return the same refusal block as `install-pii.sh:151`, and a run without a card must exit non-zero. |
| M5 | first real deploy | identity and delivery proven; two of four gates meaningless | **identity proven only with a non-default environment; survival, rates and correctness unmeasured** | `PII_RUNTIME_DIR=/run/user/1000/pii pii health` → `pii 0.1.0 (57129b83a0d4)`, `client: 57129b83a0d4`; `since_deploy.posted: 1`; unit `active`, `NRestarts=0`, `Linger=yes`, enabled | (1) **plain `pii health` on the host fails**: `error: no daemon answered (… /run/user/1000/pii/default/control.sock: No such file …)`. `~/.config/environment.d/pii.conf` sets the variable for the user manager only; a login shell has `PII_RUNTIME_DIR=` (empty). (2) Production's config carries container paths: `~/.local/pii/pii.toml:53 trajectory_dir = "/root/.local/state/pii/trajectory"` and `:63 log_file = "/root/…"`. As a result, **both runs since deploy logged `trajectory.write.failed … Permission denied (os error 13)`** (r000000004577, r000000004578, 05:03Z), and that includes the digest-now run cited as proof of delivery. Cause: `install-pii-user.sh:44-48` rewrites `/root` → `$HOME`, but `$HOME` is evaluated in the container, where it is `/root`. The rewrite is a no-op round trip, and its own comment (line 46) shows the author saw the problem. (3) Survival so far is ~1 h, not 24 h. (4) Hermes rates are unmeasured. | `ssh lnx 'pii health'` (no env) must answer, and `ssh lnx 'grep -nE "/root\|/projects" ~/.local/pii/pii.toml \| grep -v "^[0-9]*:#"'` must print nothing. |
| M6 | reads its own logs | mechanism done, gate is a clock | **mechanism-only, unwired, and noisy (demote)** | `ssh lnx '~/.local/bin/pii incidents scan --source runtime --log ~/.local/state/pii/pii.log'` today → `starts_considered=8 incidents=10` | Of the 10: **1 real** (`refusal-repeated:channel.poll.failed count=731`, 02:08:49–03:05:45Z: Discord 429s while dev and prod polled the same channel, fixed by `3709687`). **1 ambiguous** (`run-stalled:r000000004356`, the digest cron run, which was suspended and then removed). **8 false**: `daemon-start-not-ready` on every start, including the current one, which the same scan reports as `start ts=2026-11-02T04:48:32Z time_to_notice_ms=144`. So the "clock" condition has already been met by a real incident, and **no process watched**: the only caller of `--source runtime` outside tests is `scripts/archive/exercise-pii.sh:22`. | Fix start-not-ready on this log first (it must report 0 for the 04:48:32 start). Then put the scan on a host user timer and check that it opens exactly the one real incident when replayed. |
| M7 | retire the dormant surface | decided | **evidenced, and one row is already executed** | `m7-decisions.md` rows 1–6 + 3 later rows | `ssh lnx 'systemctl is-enabled ollama; systemctl is-active ollama'` → `disabled`, `inactive`. Row 6 is done, so `m7-decisions.md:13`'s "disabling … is a deployment step for the operator" is stale. | none needed. Record the measurement in row 6. |

**What I would demote:** M0 → mechanism-only. M3 → breached. M4 → stale. M6 → unwired. M5 is a deployed daemon,
not production.

## 1.3 Premises in the brief that are wrong

Each of these is a verified contradiction.

1. **"The 7-cycle M3 experiment's economics has no recorded result."** It has one, and it is void:
   `_tmp/autonomy/experiment/results.tsv`, 9 rows, $3.04, 0 proven. The cause is in
   `experiment-harness-open-work.md:29-31`. The harness was never fixed, and 151 was built anyway, contrary to the
   stop rule.
2. **"The default worker … has never been exercised."** A real Claude worker attempted real historical incidents nine
   times (`worker-*.json`, 10–28 turns each). Most of them *claimed* a fix ("The premise holds, and I've fixed it").
   What was never exercised is a mechanical proof of any such claim. That is a different and narrower gap.
3. **"M6's gate is a clock."** The qualifying event (731 refusals in under an hour) is already in the production log.
   No process reads that log.
4. **"`pii health` names the deployed rev and the client agrees."** True only with `PII_RUNTIME_DIR` exported. The
   default operator command fails.
5. **"M4 done: installer refuses without a card."** The installer that produced production does not refuse anything,
   and it overwrites production stores with dev stores.
6. **"A cron job posts new rows to Discord — it has never fired, because no new incident has occurred."** It would
   also never fire if the record were never written, and today the record file does not exist. Silence proves
   nothing.
7. **"MQTT drives devices; the model has an `mqtt` tool."** True, but `crates/pii-tools/src/mqtt.rs:209` is
   `fn call(&self, _ctx: &Ctx, …)`: the capability context is ignored. `op=pub` publishes **any topic, with
   `retain`** (`mqtt.rs:249-252`). Whatever agent holds the tool can write retained messages to
   `tasmota/discovery/#` or switch any device, with no topic scope and no approval.
8. **Gate cache.** The brief names `_tmp/gate-target` (75 GB, which is what `scripts/autonomy/env.sh:24` and
   `modules/devbox/daemon.sh:28` use). `open-items.md` §11 names `/projects/_cache/targets` (15 GB) as *the* fix.
   Two warm caches exist, 90 GB together. The register and the wiring disagree.
9. **The status documents are internally stale.** `PATH-TO-PRODUCTION-2026-11-01.md:3` still describes production as
   container pid 75729 on `669f5803d90a`, and lines 35-43 describe a "hand-started daemon in a container with no
   systemd bus". `open-items.md:3` says "As of 2026-10-11". Hermes job `745f25ecd51a` ("pii digest check", 12:20Z
   daily, **enabled**) still watches a scheduled digest the operator removed (`pii.toml.digestoff-1793589790`,
   03:19Z). It will either report a non-delivery every day or report nothing that matters.

## 1.4 The riskiest assumption (Q5)

**The premise: "the gates measure what runs in production."** All 15 gates and all 42 selftest rows run in the
`hermes` container against the container's config. Production is a different binary location, user, config file,
runtime dir and installer, on a different machine. Every defect in §1.2 for M4, M5 and M6 is invisible to every gate
for that reason. If this premise is wrong (and today it is), then each "DONE" that cites a gate says something about
the container, not about the operator's daemon.

**A cheap test, about 5 ssh commands and $0:** a read-only *host conformance probe* that fails on any of the
following:

- (a) a `/root` or `/projects` path in `~/.local/pii/pii.toml` outside comments;
- (b) a store path under `/run/`;
- (c) `pii health` failing from a login shell with no env;
- (d) any `trajectory.write.failed` or `*.failed` error-level event since the current `daemon.started`;
- (e) `incidents scan --source runtime` reporting `daemon-start-not-ready` for the current start.

Today it would be **red on all five**. The falsifier: if it comes back green on today's host, then I am wrong.

**The runner-up is the autonomy thesis:** "a model can turn an incident into a mechanically proven fix." M3, the
budget lane and the claim that the operator can stay out of the loop all rest on it. It has **zero valid
measurements** after $3.04, and the experiment that would measure it is one harness fix away (§2.2).

## 1.5 The loop's autonomy, measured (Q3)

All commits carry one author name by policy, so authorship says nothing. The loop's own commit subject is the marker:
`"<spec>: automated pass — <title> (… gate green)"` (`scripts/autonomy/cycle.sh:460`). Window: 2026-10-23 → 2026-11-02.

| who | measure | number | source |
|---|---|---|---|
| **the loop** | commits | **34 of 309 (11 %)**, and **0 of 62 since 10-31** (last on 10-30) | `git log --grep='automated pass'` |
| the loop | real cycle starts → commits | 278 → 35 (**12.6 %**). 21 gate FAILED, 16 verify FAILED, 11 parks. | `_tmp/autonomy/loop-events.ndjson` |
| the loop | outcomes since 11-01 (surviving logs) | **queue-empty 2 431**, dirty-tree 497, window-closed 68, done 56, gate-failed 40 | `grep OUTCOME= _tmp/autonomy/logs/cycle-*` |
| the loop | queue rows it wrote | **0** | `git log -p scripts/autonomy/plan.tsv` |
| **the orchestrator** (an interactive model session) | commits | **275 (89 %)** | 309 − 34 |
| the orchestrator | queue rows written by hand | **83**, plus 10 `add-row*.py` scripts dated 10-24…11-01 | `plan.tsv` history, `_tmp/autonomy/` |
| the orchestrator | loop pauses | 84 of 85 | `operator-acts.log` |
| **the operator** | recorded decisions | **10**, all dated 2026-11-02 (`m7-decisions.md:8-16` plus the broker). **0 recorded from 10-23 to 11-01.** | grep |

**The answer.** It is not a person in a robot costume. It is a robot doing the work by hand, with an executor hanging
off it.

- The operator is genuinely far from the loop. That part of the north star holds: about 10 recorded decisions in ten
  days, all made in one sitting.
- The loop is not autonomous. It chooses no work, writes no rows, and has committed nothing in three days.
- Since 11-01 it has mostly reported `queue-empty` (≈75 % of outcomes) or `dirty-tree`, which means the orchestrator
  was working in the same tree.

The 89 % is an interactive session committing straight to `master`.

**The single change with the largest effect on the ratio:** let a *detected incident* write the queue row. Today the
loop starves because only the orchestrator writes rows. The detector (M1/M6) already produces real incidents, for
example the 731× refusal. A row generated from an incident, worked on a branch, ending in a card, is the only path on
which the loop originates work. **It is gated by the M3 verdict.** If a model cannot prove a fix (0 valid of 9 so
far), generated rows will just be parked. That is why f₂ in §2.2 comes before this.

A cheaper second change: give the orchestrator its own worktree. That removes the 497 `dirty-tree` refusals and makes
the remaining ratio honest.

---

# Part 2 — The path, repaired

## 2.1 What a production personal-information service needs that no milestone asks for (Q2)

| need | state | evidence |
|---|---|---|
| **Backup of the entity store** | **missing** | No pii backup job on the host. The user timers are `amem-backup`, `amem-no-loss-gate`, `amem-consolidate-health`, `amem-metrics-append`, `dm-site`, and none for pii. `deploy/rollback-pii.sh:24-55` restores files from a `.old-*` aside dir (binaries and config), not stores. `memory.redb` is 12.6 MB and `store.redb` is 18.8 MB, each with exactly one copy. |
| **Restore drill** | **missing** | No script restores a store into a scratch prefix and answers `pii health` against it. |
| **Durable state actually on disk** | **missing** | `channel.redb` (cursor; `crates/pii-store/src/channel.rs:1-4`: "a crash cannot duplicate or lose a message") and `timer.redb` ("durable timer wheel", `timer.rs`) are on tmpfs (`daemon/mod.rs:201,204`). |
| **Production data protected from dev** | **missing (hazard)** | `install-pii-user.sh:52-54` copies dev stores over production's on every run. |
| **Store schema upgrade / rollback** | **half-present** | Every store carries `format_version=1` (`pii-store/src/lib.rs:133`, `identity::ensure_format_version`). There is no migration function and no downgrade path. A schema change today means refusing to open or losing data. |
| **Config conformance to the host** | **missing** | Container paths in host config (§1.2 M5). No check compares the deployed config's paths to the host's layout. |
| **Credential rotation** | **missing** | The Discord token is copied from the container's `pii.env` (`install-pii-user.sh:50`). There is no rotation runbook (`grep -ri rotat _docs deploy` finds only a research note). Two daemons historically held the same credential, which caused the 731×429. |
| **Auth for an opened port** | **half-present** | `pii-web` binds loopback by default and has **no auth** off-loopback (`crates/pii-web/src/main.rs:11-12` says so). The `webhook` channel requires a shared secret on a wider bind (`crates/pii-channels/src/webhook.rs:9`). Nothing stops `pii-web` being bound wide. |
| **Actuation safety (MQTT)** | **missing** | The tool ignores its capability context (`pii-tools/src/mqtt.rs:209`) and takes any topic and `retain` (`:249-252`). The input surface is a Discord forum, and the model is a 3B local model (`granite-4.2-3b`). No device allowlist, no approval for actuation, no rate limit. |
| **Broker relationship failure** | **half-present** | pii is a client of mosquitto (`0.0.0.0:1883` on lnx, listening). It has a gate row for being exercised. Not measured: behaviour when mosquitto restarts while pii is running, and whether the broker accepts anonymous publishes from the LAN. |
| **Multi-client access** | **covered by design** | A 0600 control socket for a single user. Fine for a personal service. |
| **Support path when the operator is away** | **missing** | Incidents reach Discord only from builder logs. Production faults (EACCES on every run, a 429 storm) reached no one. `pii health` fails from a plain host shell. |
| **Reboot survival** | **unexercised** | `Linger=yes` and `enabled` are correct. The host has been up since 2026-09-16, so no pii install has seen a reboot. |
| **Log retention** | covered | `--log-rotate-bytes 10485760 --log-keep-files 5` in the unit. |

**Missing milestone:** between M4 and M5 there should be an **M4½, "host conformance"**: the probe from §1.4 as a gate
row, plus backup with a restore drill, with every durable store on disk. M5 cannot be honest without it.

## 2.2 Where the leverage is (Q4)

Ranked by expected value per unit of risk:

1. **(f₁) Host conformance probe plus the five host fixes.** Highest value: these are measured defects in the running
   product. Risk is low: the probe is read-only, and each fix is config or installer work with a control. **Do this
   first.**
2. **(f₂) Fix experiment harness defect #6, then re-run the 7 workers.** Model spend is $0 for the fix and ≈$3 for the
   re-run (operator must approve). The value is high, because it decides whether M3 and the autonomy budget lane
   should exist. The splice control in `experiment-harness-open-work.md:40-43` keeps the risk low.
3. **(e) Consolidate the two gate caches.** Disk pressure is the measured cause of false reds (`open-items.md` §11).
   Value is medium and risk is low, **but** per the house rule this means moving (not deleting) the 15 GB `_cache`
   one aside after confirming nothing points at it. Freeing the 75 GB one is the operator's decision. `mv` into
   `_trash` frees nothing for 30 days (the retention job `f6307338f23d`).
4. **(a) One $5 real-worker dispatch through `incident-to-branch.sh`.** Low value **until f₂ lands**. With the base-run
   defect unfixed it cannot produce red→green, so it would repeat a void measurement at a higher price.
5. **(d) amem health honest about writes.** Off this path. amem is a separate service. Worth doing, but not for pii's
   production.
6. **(b) Wait for M6's clock.** Zero or negative value as things stand. The event has already happened, and nobody
   is watching. Waiting creates a false "no incident" week.
7. **(c) Reconcile the 76 plan rows.** Constrained by the brief and low value. Leave them frozen.

**What would falsify "do f₁ first":** the probe comes back green on today's host, or the five defects turn out to be
harmless. For example, if `trajectory_dir` is unused by any production function, then the EACCES is noise rather than
lost evidence. Check: `crates/pii-server/src/engine/learn_methods.rs:178` uses the trajectory root for promotion
outcome measurement, so the learning loop (specs 123–125) measures against an empty directory on production.

## 2.3 What to cut (Q6)

"In the binary" means a normal dependency of `pii-cli`, the crate that builds `pii`.

| item | in the `pii` binary? | activity since 10-15 | verdict |
|---|---|---|---|
| `pii-web` (599 LOC) | **no**: nothing depends on it | 0 commits, last 10-11 | **dormant cargo.** It also carries the unauthenticated off-loopback surface (§2.1). |
| `pii-evolve` (793) | **no** | 1 commit | **dormant cargo** |
| `pii-mcp` (496) | **no** | 3 commits, one on 11-02 (`c78ae56`, +1 line, for the mqtt tool) | **unclear.** Someone keeps it compiling. The operator must say whether an MCP client uses it. |
| `pii-wasm` (1 189) | yes (`pii-server/Cargo.toml:20`) | 0, last 10-10 | dormant inside the binary |
| `pii-tui` (1 088) | yes (`pii-cli/Cargo.toml:27`, `cmd_tui.rs:100,134`) | 0, last 10-10 | dormant inside the binary; cheap to keep |
| `pii-hal` (1 404) | yes (`pii-server/Cargo.toml:19`), gpiod behind feature `hal-gpiod` | 0, last 10-10 | **operator decision "postponed"** (`m7-decisions.md:12`). Per `m7-decisions.md:20`, postponed code stays compiled. Do not cut. |
| `pii-uds`, `pii-worker` | yes | 0–1 | **load-bearing** (client dial, worker spawn) |
| `testkit` | does not exist | — | do not build. `open-items.md` #6's own condition (a second crate duplicating the helpers) is unmet. |
| laya lane | not a crate: a branch (`autoresearch/gate-lat2`) plus specs | — | off the production path. Leave it parked. |
| 350 specs | — | 96 added in the window; **251 of 350 have no Closure section** | do not bulk-rewrite. Stop adding specs that are not on the §2.4 list. |

Load-bearing for the path: `pii-cli`, `pii-server`, `pii-store`, `pii-channels` (discord and mqtt), `pii-tools`,
`pii-memory`, `pii-graph`, `pii-model`, `pii-observe`, `pii-core`, `pii-proto`, `pii-net`, `pii-uds`, `pii-worker` and
`pii-learn` (incidents).

**A retirement order that cannot break production**, each step its own spec:

1. **`pii-web`, then `pii-evolve`.** Remove them from the workspace members, `mv` the dirs to `_trash/` (never delete),
   and do not redeploy. Proof: `cargo tree -p pii-cli -e normal` is byte-identical before and after, and the gates are
   green. Production cannot change, because the binary's dependency graph does not change.
2. **`pii-mcp`**, only after the operator confirms no MCP client uses it. Same proof.
3. **`pii-wasm`** behind a non-default feature. This changes the binary, so it needs a card, a redeploy and the §1.4
   probe. Proof: `cargo tree -p pii-cli -e normal | grep -c pii-wasm` → 0, and `pii health` shows identical stores and
   mounts.
4. `pii-hal`, `pii-tui` and the postponed channels: **not cut.** They are operator decisions, recorded.

## 2.4 The next seven days (Q7)

Each item: the action, then what proves it worked (✓) and what proves it did not (✗). Each needs its own spec and
gate before anyone builds it.

| day | action | ✓ proves it worked | ✗ proves it did not |
|---|---|---|---|
| 1 | **Spec and land the host conformance probe** (read-only, 5 checks from §1.4) as a gate row that runs over ssh | red on today's host on all 5, each naming the line; a control copy of a corrected config turns (a) green | it is green today, or a check passes with ssh unreachable (it must refuse, not pass) |
| 1–2 | **Fix the installer**: compute the remote `$HOME` remotely; no store copy over an existing production store (refuse if the target exists); require the deploy card like `install-pii.sh:151` | redeploy via card → probe (a) green; `ls ~/.local/state/pii/trajectory/` holds `r…jsonl` after one `digest-now`; installer without a card exits non-zero | a `trajectory.write.failed` after the redeploy; the store sizes change across the install |
| 2 | **Durable stores off tmpfs**: `channel.redb` and `timer.redb` under `~/.local/state/pii` | probe (b) green; `systemctl --user restart pii` preserves the channel cursor (no re-processed message) | any store path still under `/run/`; a duplicated post after restart |
| 2 | **`pii health` from a plain shell**: the client finds the daemon's socket without env (the default path should match the unit, or the unit should use the default) | `ssh lnx '~/.local/bin/pii health'` rc 0 | "no daemon answered" |
| 3 | **M6, made real**: fix `daemon-start-not-ready` against today's production log, then add a host user timer that runs the runtime scan every 15 min and publishes to the same incident surface | replay of today's log → `refusal-repeated:channel.poll.failed` 1, `daemon-start-not-ready` **0**; the timer's first run writes a row; a fixture with a real unready start still opens one | any start-not-ready for a start whose `time_to_notice_ms` is printed; or the publisher silent when the record file is missing |
| 3 | **Make the Discord publisher loud** when its record is absent (`pii-incidents-discord.sh:9`) | with `PII_INCIDENTS_MD=/nonexistent` it prints a "record missing" line | it exits 0 silently |
| 4 | **Backup and restore drill**: a nightly user timer copies the four redb stores to a dated dir; a drill restores one into a scratch prefix and runs `pii health --store …` | the drill's `format_version=1` and byte counts match; two nights of dated dirs | a drill reading the live store (it must be a copy); a backup taken mid-write without the daemon's consistency (measure redb copy semantics first) |
| 4 | **Fix experiment harness defect #6** with the splice control ($0) | the control row (fix applied to base too) goes GREEN on base; a known-good historical fix goes RED on base and GREEN on fix | base rc=0 with 0 tests run, again |
| 5 | **Operator-approved re-run of the 7 workers** (≈$3, ceiling $5) | a pre-registered verdict: ≥3 proven → M3 stands; <3 → freeze the default-worker path of 151 (keep the code) and record M3 as "evidence pack for a person" | any row whose base run executed 0 tests |
| 5 | **MQTT scope**: the model's tool gets a device allowlist and `send` only (no raw `pub`, no `retain`); the CLI keeps full power | a test where the model's tool is asked to publish `retain=true` to `tasmota/discovery/x/config` → refused; `send` to `SW-FAN-TV` still works | the refusal test passes because the tool is absent (test that `send` succeeds in the same test) |
| 6 | **Retire superseded watchers**: disable Hermes job `745f25ecd51a` (digest check). This is the operator's call (§3). | no 12:20Z line tomorrow | — |
| 7 | **Re-score M5** from a 24 h window after the last redeploy: unit uptime, `hard_deaths: 0`, probe green, Hermes call/error rates against the 161/30 d baseline | all four measured with numbers | any leg reported without a number |

**What not to do this week:** spend on (a) before the harness fix; build the broker; add telegram; rewrite the 76
rows; count M6's week before the scanner is wired; delete either gate cache without the operator; reboot `lnx`
without the operator (it runs mosquitto, llama.cpp and Hermes); start any new spec that is not on this list.

**If the answer were "measure first, change nothing":** run the §1.4 probe and the replay from day 3. If the probe is
green, or the replay shows the 8 start alarms are true, I would withdraw days 1–3 and move f₂ first.

---

# Part 3 — The operator's decision

One line each: the decision, its options, and their consequences.

1. **May production stores and trajectory move to `~/.local/state/pii` (off tmpfs)?** Yes: one redeploy, and state
   survives reboot. No: a host reboot silently resets the channel cursor and timers.
2. **Where do backups go?** Same disk: protects against a bad deploy, not against disk loss. Another host or volume:
   real protection, plus one more credential or mount to own.
3. **What may the model do to the house?** (a) full MQTT as today; (b) allowlisted devices, `send` only;
   (c) actuation needs a Discord confirmation. Today's input is a forum and a 3B model, so (a) is the risky one.
4. **M3: accept that 151 was built against its pre-registration.** Either approve ≈$3 (ceiling $5) for the re-run after
   the harness fix, or freeze 151's real-worker path now and treat incident resolution as yours.
5. **Retire Hermes job `745f25ecd51a` (digest check)?** Yes: one less daily line about a schedule you removed. No: it
   reports non-delivery of a thing you decided not to deliver.
6. **The 75 GB gate cache.** Keep it (fast gates, 88 % disk) or free it (one cold ~28 GB rebuild, more headroom). The
   15 GB `_cache/targets` duplicate should go first.
7. **A reboot drill of `lnx`.** When, and in which window. This is the only way to prove reboot survival, and it
   interrupts mosquitto, llama.cpp and Hermes.
8. **Is the Discord forum the support path when you are away?** If yes, production incidents must reach it (day 3).
   If no, name the phone channel.

**What I would tell you to decide today:** **1 yes, 3 (b), 4 approve after the harness fix.** These three decisions
change what runs on your machine and what it is allowed to do. Everything else can wait a week.

---

## The single next action

**Write and run the read-only host conformance probe** (§1.4: five checks over `ssh mvcorrea@10.10.10.217`). It should
be specced as a gate row with a control.

- **Cost:** $0 model spend, about one Sonnet implementation pass, no change to production.
- **Gate:** it must be **red on today's host on all five checks**, each naming the offending line or path. A corrected
  config copy must turn check (a) green. With ssh unreachable it must **refuse** (exit 2), not pass.
