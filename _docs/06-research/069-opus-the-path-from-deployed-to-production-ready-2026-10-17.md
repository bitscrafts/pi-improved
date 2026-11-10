# 069: The path from "deployed" to "production ready" (Opus, 2026-10-17)

This is a read-only review. The only repo file written is this one, and nothing was committed. One scratch
directory, made with `mktemp`, is listed at the end. No signal was sent to the daemon. Evidence labels:
- **C-run**: I ran it today, 19:40-20:10 UTC.
- **C-read**: I read the lines.
- **asserted**: from a document or the brief.

_Written incrementally._

## 0. First: the fix is not in the committed tree

The brief says 175 is "FIXED IN THE TREE (gate green)". **In the working tree, yes. In git, no.**

| fact | evidence | label |
|---|---|---|
| `7f98624` (175's automated pass) changes **only** `crates/pii-cli/src/cmd_daemon.rs`. Its new line 386 calls `pii_server::resolve_graph_dir(&runtime_dir, &store_path)`. | `git show --stat 7f98624` | C-run |
| **`resolve_graph_dir` is defined nowhere in HEAD.** `git grep -n resolve_graph_dir HEAD -- crates` returns only the call site. The definition (+16 lines) sits **uncommitted** in `crates/pii-server/src/lib.rs`. | `git show HEAD:crates/pii-server/src/lib.rs \| grep -c 'fn resolve_graph_dir'` → **0**; working tree → **1**; `git diff --stat` → `lib.rs \| 16 +` | C-run |
| 175's first exit criterion, `cargo test -p pii-server --test daemon_graphs`, runs a file that is **untracked**: `?? crates/pii-server/tests/daemon_graphs.rs` (37 lines). | `git status --short` | C-run |
| 175's second criterion, `grep -q graphs crates/pii-cli/src/cmd_daemon.rs`, **was already true before the fix**: the pre-fix file has 1 match (`graph_dir: runtime_dir.join("graphs")`). It cannot fail. | `git show 7f98624~1:… \| grep -c graphs` → 1 | C-run |
| Row A42's commit-paths column is `crates/pii-cli/src/cmd_daemon.rs`, a single path. It cannot stage the library function or the test, which is **the defect 066 §0 predicted for hand rows, now in a loop row**. | `plan.tsv` | C-run |
| The gate that went green ran on the **dirty main tree**: the item boundary's quick gate runs in place. So it compiled the uncommitted `lib.rs`. | 067 §Q1 (the quick gate runs in place); this table | C-read |

**Consequence:** HEAD (`ba554ec`) does not build `pii-cli`, so a deploy from a clean checkout of HEAD cannot
happen. §0.1 has the confirming build: rc=101.

### 0.1 Confirmed by a clean build (C-run)

`git archive HEAD` (ba554ec) into `<scratch>/rev069-8e5o`, then `CARGO_TARGET_DIR=<scratch>/rev069-8e5o/tgt cargo check -p pii-cli`:

```
error[E0425]: cannot find function `resolve_graph_dir` in crate `pii_server`
386 |     let graph_dir = pii_server::resolve_graph_dir(&runtime_dir, &store_path);
error: could not compile `pii-cli` (bin "pii") due to 1 previous error
rc=101
```

**HEAD does not build the daemon binary.** No release can be cut from history until `lib.rs` and the test are
committed.

### 0.2 Second: once the graphs load, the digest re-posts in a loop

This is C-read and then **C-run**, and it is the most important finding of this round.
- `CronEntry.schedule` is a **"Display-only schedule mirror"** (`crates/pii-server/src/daemon/status.rs:34-35`).
  Scheduling is the job of a `timer` node at the cron graph's entry. `config.rs:457`: "its entry must be a `timer`
  node carrying the cron".
- `on_finished` (`daemon/mod.rs:455-468`) starts the next cron run **immediately** on any terminal, with no
  backoff, on the assumption that the entry timer then suspends it until due.
- **`discord-digest.toml` has 0 `timer` nodes**, both installed and in the repo (`grep -c 'kind = "timer"'` → 0
  and 0). Its own header says "To run it daily, put a `timer` node … in front (F5.8)". Nothing enforces the
  precondition.
- **Today this is masked**, because the graph cannot be found, so `start_run` fails and nothing re-arms.
- **The moment 175 is deployed, it is unmasked:**
  - `start_cron_entries` starts the digest at boot;
  - it posts "pii digest: 0 device(s) known, 0 lesson(s) remembered" (`memory_entities: 0`);
  - it finishes, and `on_finished` starts it again.
- Each run has a new `key = "digest-${run}"`, so idempotence does not stop the repost. The `to` is the forum id,
  so each post opens a **new forum thread**. **Predicted: a post storm into the operator's forum, bounded only by
  Discord's rate limit.** After that, a hot loop of refused runs, because a failed terminal re-arms too.

**Measured (C-run).** I used the **installed** binary (`~/.local/bin/pii`, rev `0e34719`, the same `on_finished`)
and a scratch runtime dir with `graphs/loop.toml`: one `transform` node and no timer. One `[[cron]]` with
`schedule = "0 9 * * *"`, no channel, no network, under `timeout -s KILL 8`.
- `pii health` read **`runs: 7`, then `runs: 146` 2.4 s later: about 58 runs per second**, against a schedule that
  says once a day.
- A first 6 s run had reached run id `r000000000687`.
- Both scratch daemons exited, and the production pid 870350 was untouched.

With the digest's `post` node in place of `transform`, each of those runs is a Discord forum post attempt.

**This turns "redeploy carries the fix" into "redeploy arms a spam loop", unless the graph or the daemon changes
first.**

### 0.3 Still true at HEAD (C-run)

- `start_cron_entries` still discards a failed start (`if let Ok`, `daemon/mod.rs:225`), and the router still
  `continue`s (`router.rs:120`). 068 §3 item 2 has not been done.
- 175 adds a `daemon.graphs_fallback` warning when it falls back. It adds nothing for a refused run.
- Production at 19:45 UTC: revision `0e34719dce0d`, up 15.95 h, `runs: 0`, `channel_posts: 1`, and
  `/run/user/0/pii/graphs` still absent.

## 1. The path, ordered by what earns the verdict

Each step closes on a product counter read from a running daemon, never on a document (068 §5). Steps 1-3 are
preconditions of the redeploy, step 4 is the redeploy, and steps 5-7 earn the verdict.

| # | step | why it is here | the proof that closes it |
|---|---|---|---|
| 1 | **Commit 175 whole:** `crates/pii-server/src/lib.rs` (`resolve_graph_dir`) and `crates/pii-server/tests/daemon_graphs.rs`. Fix row A42's paths to the spec's files. | HEAD does not build `pii-cli` (§0) | on a **clean export** of HEAD: `cargo check -p pii-cli` rc=0 and `cargo test -p pii-server --test daemon_graphs` rc=0; `git status --short` empty |
| 2 | **The digest cannot loop.** Give `discord-digest.toml` an entry `timer` node with `cron = "0 9 * * *"`. **And** make the daemon refuse at start a `[[cron]]` whose graph entry is not a `timer`, enforcing `config.rs:457`'s documented precondition. | §0.2: 58 runs per second, measured | the scratch reproduction in §0.2, re-run on the new binary: `runs` stays ≤ 1 over 10 s (the timer suspends). A timer-less cron graph makes the daemon refuse to start, naming the entry. |
| 3 | **Refusals are loud** (068 §3 item 2): `start_cron_entries` and `poll_once` emit `cron.start.refused` / `channel.run.refused`; on-finish re-arm gets a minimum interval. | the class that hid 14 h of zero runs | a scratch daemon with an empty graph dir logs `cron.start.refused` within 1 s, and `pii incidents scan --source runtime` opens `refusal-repeated:cron.start.refused`. HEAD logs nothing. |
| 4 | **Redeploy through the card**, from a clean tree whose revision has a full (fresh-export) gate PASS | the card's own rule (runbook §"Gate tiers": the card must quote the `--fresh` verdict) | §2 below |
| 5 | **The first digest**: one post at 09:00 UTC, exactly one | it proves the scheduled path once | 09:01 UTC: `runs == 1` for the digest, `channel_posts == 2` (start + digest), and **no** second digest before 09:00 the next day |
| 6 | **The soak, 7 days**: M5b and M5c | the plan's own Production criteria | over 7 × 24 h: `channel_posts ≥ 8` (7 digests + ≥ 1 start); runs grow by exactly 1 per day plus inbound; 0 `cron.start.refused`; every `daemon.started` either paired with a `daemon.stopped` or followed by a runtime incident; uptime of one unit ≥ 24 h at least once |
| 7 | **The `pii-mcp` swap** (M5d), behind its `tools/list` probe | the one surface with real daily use runs the 10-15 binary | Hermes `mcp__pii__*` error share over the 7 days after ≤ the 7 days before (read-only `state.db` query, 059 §1.1) |

**Explicitly not on the path:** coverage, the loop's own rows, ingress on a forum thread (165), and anything in M3.
They improve the machinery; they do not earn "production ready".

## 2. The redeploy: before and after

**Before the daemon on the host is replaced, all of these must hold. Each is one command, and each is red today.**

```sh
# B1  the committed tree builds and holds the fix (red today: resolve_graph_dir only in the working tree)
git status --short                                   # → empty
git grep -c 'fn resolve_graph_dir' HEAD -- crates/pii-server/src/lib.rs   # → 1
git ls-files crates/pii-server/tests/daemon_graphs.rs                     # → the path
# B2  the digest graph is scheduled by a timer (red today: 0)
grep -c 'kind = "timer"' graphs/discord-digest.toml  # → ≥ 1
# B3  a full fresh-export gate PASS on that exact revision (the card quotes it)
bash quality-gate.sh --fresh                         # → "fresh-gate: PASS", rev == git rev-parse HEAD
# B4  the loop reproduction on the NEW binary does not loop
#     (§0.2 scratch: one timer-less cron graph → the daemon refuses; the timer graph → runs ≤ 1 in 10 s)
# B5  the rollback target exists and is the current good binary
ls -la ~/.local/bin/pii    # the 0e34719 build, to be moved aside by install-pii.sh (never deleted)
```

**After `install-pii.sh` and the unit restart.** The deploy carried the fix if and only if:

| # | command | the number |
|---|---|---|
| A1 | `PII_RUNTIME_DIR=/run/user/0/pii pii health --json \| jq -r .build.revision` | the new short rev, no `-dirty`, equal to the card's |
| A2 | `jq 'select(.kind=="daemon.graphs_fallback")' ~/.local/state/pii/pii.log` | **one** event at the new start, `using` = `~/.local/state/pii/graphs`, **or** zero events and `/run/user/0/pii/graphs` present (the unit's link) |
| A3 | `pii health --json \| jq .counts.runs`, 60 s after start | **0 or 1**: the digest's run suspended in its timer. **Any value ≥ 2 within the first minute is the §0.2 loop, so roll back at once** (`bash deploy/rollback-pii.sh`). |
| A4 | the same, at 09:01 UTC | runs grow by **1** (the digest), and `channel_posts` grows by exactly 1 |
| A5 | `pii incidents scan --source runtime --log ~/.local/state/pii/pii.log` | no new `refusal-repeated:*` after the start |
| A6 | `grep -c daemon.started` against `grep -c daemon.stopped` in the new log, over 24 h | equal, or each excess has a runtime incident |

**The single number that says "the deploy carried the fix": `counts.runs` = 1 at 09:01 UTC on the first morning,
and = 2 at 09:01 UTC on the second.** Anything else means it did not.

## 3. What 068 would say now, on today's tree

| | finding in 068 | now |
|---|---|---|
| **(a) fixed in the tree** | graphs looked for in a directory that does not exist | **fixed in the working tree only.** 175's commit calls a function that is not committed (§0). Not fixed in git. |
| (a) | the reader opens one incident from a repeated refusal | still true at HEAD (`300b810`, unchanged) |
| (a) | a parked head freezes the queue | `ba554ec` (169), green per the loop. Not re-run by me. |
| **(b) still true in production** | `runs: 0`, no digest, `channel_posts: 1` | **all three**, at 19:45 UTC (§0.3) |
| (b) | revision `0e34719`; `pii-mcp` not swapped (`pii-mcp.new` staged) | unchanged |
| (b) | "kill -9 → 8 s" rests on a catchable-signal restart; the supervisor is not visible from the container | unchanged; no new evidence |
| (b) | refusals discarded silently (`mod.rs:225`, `router.rs:120`) | unchanged **at HEAD too** |
| (b) | coverage not measured on the deployed rev | unchanged |
| **(c) newly true because of today's fixes** | — | **the digest re-post loop is armed by 175.** Today it is masked by the missing graphs. Deploying 175 unmasks a measured 58 runs/s re-arm against the operator's forum (§0.2). |
| (c) | — | a fix can now pass the loop's gate and be committed in a form that does not build. The quick gate runs on the dirty tree, and the row's paths do not cover the files the fix needs (§0). The loop has produced its own `HEAD`-red. |

**068's verdict on today's tree is still: deployed, not production-ready.** It now carries a sharper caveat: the
one change that would make the product run has to be made safe *before* it ships.

## 4. The remaining risk, in writing, for whoever operates this

> The daemon on the host has run nothing since it was deployed, and the committed fix for that does not build yet,
> because half of it is still uncommitted. Once that fix ships, the daily digest will re-run about 58 times a
> second and flood the forum, because its graph has no timer and nothing refuses a timer-less cron graph. Do not
> redeploy until both are fixed, and roll back at once if `pii health` shows more than one run in the first
> minute.
## 5. What this project still cannot see about itself

**It cannot see the difference between its working tree and its history.** Every verdict today was taken on the
tree, and every claim was made about git.
- **The gate runs in place.** The item boundary's quick gate compiles the main tree, uncommitted files included,
  so "gate green" certifies a tree that no commit contains. 175 is the proof: the gate was green, the commit is
  `7f98624`, and `git show HEAD:…/lib.rs` has no `resolve_graph_dir`.
- **Leftovers are the symptom, counted and ignored.** There were 5 "leftover from the previous item" commits
  between 18:00 and 18:40 today (`ed3ae7d`, `2f231af`, `80844e3`, `4b37f34`, `47979fd`). Each one is a diff that
  existed in the tree, passed or failed a gate there, and reached history by another door.
- **No instrument asks "does HEAD, exported clean, build?"** The fresh-export gate exists (`--fresh`), but it runs
  only before a deploy, and nothing runs it after a loop commit. The ledger 067 proposed would carry `rev` and
  `tree_dirty`, and nothing writes it yet.

**The second blind spot is the same shape: it cannot see a behaviour it does not emit.**
- A cron start that fails is discarded (`if let Ok`).
- A re-arm loop with no timer is indistinguishable from a busy daemon: every run succeeds, every health
  subsystem is `ok`, and `runs` just grows.
- Nothing compares a counter with its **expected rate**: one digest per day, so `runs` per day ≈ 1 plus inbound.
  58 per second and 0 per 14 hours are both "green".

**The instrument that closes both, precisely:**
1. **A post-commit export check:** after every loop commit, `git archive HEAD | cargo check --workspace` in the
   fixed export. A red result is `BASE RED build` (067 Q5) and stops dispatch. **Proof:** replayed on `7f98624`, it
   is red; on HEAD, once step 1 of §1 lands, it is green.
2. **Expected-rate assertions on product counters**, declared next to the config that implies them:
   `[[cron]] expect_runs_per_day = 1`, `[[channel]] expect_posts_per_day = ">=1"`. The runtime reader opens an
   incident when the observed rate over 24 h falls outside ×0.5-×2 of the declared rate. **Proof:** today's
   production (0 runs in 16 h) opens `rate-below:daily-digest`; the §0.2 scratch loop opens
   `rate-above:probe` within one minute.

## NOT VERIFIED

| item | why |
|---|---|
| the `pii/` keys in the workspace memory written today | no the workspace memory CLI or store reachable from this shell (`command -v the workspace memory` → none) |
| the supervisor on the host, and SIGKILL survival | not visible from the container (068 §1), and I did not signal production |
| 169's green gate | taken from the commit message; not re-run |
| the Discord-side behaviour of the loop (429s, forum threads per post) | inferred from §0.2 plus `send_tagged`'s forum shape; deliberately not reproduced against the real channel |

## Scratch and repo state

- Scratch: `<scratch>/rev069-8e5o` (718 M) holds the clean export, its check target, and the two re-arm reproductions
  (`rearm/`, `rearm2/`). I created it with `mktemp -d` in this run, and it can be removed. Nothing in it is
  referenced from the repo.
- Repo: nothing changed except this file, and nothing was committed. The uncommitted `crates/pii-server/src/lib.rs`
  and the untracked `crates/pii-server/tests/daemon_graphs.rs` are the loop's: they are the missing half of 175
  (§0), and I left them exactly as found.
