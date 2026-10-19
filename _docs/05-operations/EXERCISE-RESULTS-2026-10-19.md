# EXERCISE RESULTS — 2026-10-19 (overnight pass)

Every line: exercise id, what was injected, what pii did BY ITSELF, PASS/FAIL, evidence.
Rule of the exercises, verbatim from the user: **"you just observe, the system should heal"** —
the harness injects and reads; only pii repairs. No verdict was reported over a process that did
not start; three harness drafts were discarded when the harness itself was the defect (noted inline).

Deploy measured this morning: deployed binary was **stale** (built 2026-10-18 21:46, revision 4182856bcc1b —
predates the 191 conn.rs fix committed 06:44Z) while HEAD said the fix was in. Measured by sha256 and
revision stamp, not by reading code. Fresh binary built from HEAD, sha256
`4f13db5aeaec9be0e7780aa1be87a3b1734a74e2c0cceab2fe3df23735272d75`, revision stamp `121fb6de67e1`,
swapped in and daemon restarted 07:07:22Z (old binary aside at `/root/.local/bin/pii.old-1792393586/`).

| # | Injected | What pii did by itself | Verdict | Evidence |
|---|----------|------------------------|---------|----------|
| E1 | `kill -9` the LIVE daemon (pid 2638247) | lnx systemd restarted it: new pid 2642220; `daemon.started` logged (pid 2642220); the armed cron run stayed EXACTLY 1 (`r000000004129`) across the death — before/after lists byte-identical | PASS | `run ls --status suspended` before == after (1 row); pii.log last line ending the old process is a bare close (no `daemon.stopped` — a hard death leaves none), next line `daemon.started … pid 2642220`; unit `active`. **Latency corrected by re-reading the log 2026-10-19 09:05Z**: the old process's last line is 07:13:18.597Z and the replacement's start marker 07:13:50.225Z — the log supports an upper bound of **31.7 s**, not the `≤14 s` written here earlier (that was the harness's own wait, not a measurement). The mechanism claim stands; the latency figure did not carry. |
| 187 | four hard kills on a SCRATCH daemon | every restart found the suspended run and kept ONE armed run (it grew to 4 under the pre-187 binary) | PASS | `scripts/exercise-187-restarts.sh`: *"VERDICT: 187 HOLDS — four starts, one armed run"*; workspace kept at `/projects/_tmp/exercise187-WRHfTv` |
| E2 | the runtime-graphs symlink REMOVED (installed `<state>/graphs` left); then a hard kill; then, additionally, BOTH graph paths moved away | daemon logged `daemon.graphs_fallback` naming the installed path (`using":"…/ex19-VWAYFn/graphs"`), armed the same ONE run again (`cron.rearmed` ×1, no duplicate), kept answering health with NO graph path at all, refusing each cron start by name | PASS | `scripts/exercise-2026-10-19.sh` run 2: fallback=1, armed_after=1, rearmed=1; E2-EXTRA: health answered with both paths gone |
| E3 | a `[[cron]]` naming graph `no-such-graph` (scratch) | `cron.start.refused` at arm time naming cron `broken` and graph `no-such-graph` (`not found: graph 'no-such-graph'`); the good cron stayed armed (suspended=1) | PASS | same run, event line quoted in the script output; control: no fallback event while the runtime path existed (false-positive control) |
| E8 | `pii memory put` of a sentence with the nonce `zebra-quartz-lantern` (LIVE daemon) | hybrid search answered: doc id `9176c04b3182b6c177f4260ca1e5c15f` hit on **lanes bm25+vector**, overall `degraded:false` | PASS | `_tmp/autonomy/e8/put.json`, `_tmp/autonomy/e8/search.json` |
| E9 | the store's file lock held by a foreign process BEFORE the daemon starts (scratch) | the daemon REFUSED to start: `error: store op open on …/store.redb: Database already open. Cannot acquire lock.` — names the store, exits (2), no hang, no running daemon | PASS | `scripts/exercise-e9-store-lock.sh`: up=0; event.log `cli.failure` naming the path. Two earlier drafts aborted by design (holder race → the inject had to prove itself with a marker first) |
| E11 | a row parked at the queue head, a pending row behind it (scratch plan+state) | `pick` skipped the parked row AND NAMED it: `pick: skip parked X99(parked) -> dispatching A60`; control: ALL parked → `NONE` + `pick: every open row is parked … waiting for a person` | PASS | probes under `_tmp/autonomy/e11/` |
| E10 | v1: `[[cron]] schedule` two minutes ahead; v3+: the timer node's own cron two minutes ahead (the real knob, measured) | v1 false-FAIL (counts can't move: the armed run is consumed and one re-arms ~1 s later). v2 exposed the mechanism: `[[cron]] schedule` is a DISPLAY MIRROR — the wheel armed for the GRAPH's timer-node cron (journal `Suspended {in_ms:5606922}` → 09:00Z). v3–v5 taught the harness three of its own bugs (compact-line row count, raced triple reads, succeeded-only terminal grep). Final: armed 1 row → at the injected occurrence it went terminal + a NEW row suspended → rows=2 exactly through +115 s; wheel keys `r1:tick:07:58:00Z`, `r2:tick:…+24h exactly` — ONE firing, ONE re-arm armed a day out, silence | PASS | `/projects/_tmp/ex19e10-8UlGIA/` (final), `-nm7cSe` (v3), `-2Mghxp` (v4); `scripts/exercise-e10-one-firing.sh` final revision |
| E12 | a DIRTY working tree (untracked exercise scripts, modified docs) while the release is built from `git archive HEAD` with a fully dirty tree | the export-built binary was stamped with the clean 12-hex revision `121fb6de67e1` — the archive string appears in the binary and NO `-dirty` suffix exists (0 occurrences; the stamp source is `PII_BUILD_REVISION` from the export, `crates/pii-server/build.rs:28`) — the shipped revision names its commit | PASS | `/tmp/e12export-bzKjlg/` build: `PII_TRAJECTORY_DIR6da454ccd9f5…` in strings; `grep -c dirty` = 0; deploy-time precedent measured on the same revision at install (health: `revision: 121fb6de67e1`) |
| E4 | the llama base URL pointed at a dead loopback port (connection refused in ms; the real llama server untouched, via a scratch `PII_CONFIG`) | the run TERMINATED BY ITSELF in 0 s (rc=1, the 90 s timeout never fired) with a refusal naming the fault: `unavailable: net: no address reachable for 127.0.0.1`, a clean failure terminal — no hang, no crash loop | PASS | `scripts/exercise-e4-provider-refusal.sh` (twice, second with the fixed detector); output quoted in the script output |
| E5 | none needed (symptom already live: Discord poll returns 429 on its own) | the inbound channel logs `channel.poll.failed`, keeps the cursor, never crashes — AND the runtime scanner DOES open ONE incident per burst: `refusal-repeated:channel.poll.failed count=22 first=07:11:48Z last=07:14:57Z latency=7s` (5 bursts total in the log). TWO residuals keep it FAIL: (a) the poll error text carries no `Retry-After` (`unavailable: discord: poll status 429`, discord.rs:149-154) so a 429 is polled again in ~1.2 s; (b) the error backoff never leaves the store — `record_error` bumps a counter (`crates/pii-store/src/channel.rs:125`) that `router.rs` never reads back | **FIXED + EXERCISED 09:25Z (spec 194, `0c1ded6`); not deployed** — the poll error now carries Discord's body (`discord: poll status 429: {...\"retry_after\": 2...}`) and the router pauses for `retry_after` | `scripts/exercise-e5-poll-retry-after.sh` A/B, same harness both arms: deployed `121fb6d` = **72 polls in 7 s**, gaps 103–107 ms, no token in the text; fixed `0c1ded6` = **4 polls in 7 s**, gaps 2004/2076/2081 ms for a requested 2000, token present. Residual (b) CORRECTED: `record_error`'s counter IS read — by `daemon/status.rs` as `ChannelStatus.errors` (cumulative); pacing is process-local by design, spec 194 R3 |
| E6 | none (symptom already live: forum post 429) | outbound records `channel.send.failed` + `channel.post_refused` with Discord's `retry_after:132.176` IN THE ERROR TEXT, 0 since the deploy — but only `reply()`'s blind one-retry exists (`router.rs:171`); nothing READS `retry_after` to pace the sender, so "backoff respects retry_after" is not met by construction | **PASS (after spec 195)** — A/B on one harness: the deployed pre-fix binary dialled 2→3→4 times across the three runs (3 delivered, 0 paced); the fixed tree dialled 1→1→2 (1 delivered, 2 paced refusals, `paced by the provider for 2480 ms`) | `_tmp/ex195-*/state.json` + `event.log`; spec 195 Closure table; `scripts/exercise-e6-send-pacing.sh` |
| E7 | none observed live this pass | denial is per-connection in the shipping code (grant window 16 MiB in/out per connection, `crates/pii-server/src/daemon/config.rs:698`); committed tests prove an oversized response denied while later posts succeed, and that concurrent connections cannot spend each other's window — 6/6 green on the deployed tree. Historical `channel.egress.denied` (57,284, last 10-17 03:46Z) predates the per-connection fix | PASS (tests) / historical burden cleared | `cargo test -p pii-net --test budget_survives_a_day --test budget_window`: `ok. 4 passed` + `ok. 2 passed` |

Honest residue:
- The stray doc `1299bfbf0ab382c160af572b95cae4f2` (text "--help") was written into the live memory
  store by an accidental `memory put --help` (positional arg) this morning; no delete verb exists.
  Left as-is and disclosed here rather than hidden.
- E5 and E6 were the two OPEN defects at the start of this pass; both now have a spec, a committed fix with
  criteria green by hand, and a live A/B against the deployed pre-fix binary (E5 = spec 194 `0c1ded6`, E6 =
  spec 195 `05b3474`). NEITHER IS DEPLOYED: the binary carrying them needs its own card. The 12:00Z
  occurrence runs on `121fb6de67e1` and is not affected by either.
- E5's incident half was already live and is unchanged; E6's "post still lands" is proved only for windows
  shorter than the post node's own timeout — a 132 s window still fails the run and waits for the next
  occurrence (a deliberate bound, spec 195's Closure).

## A production finding at 09:00Z — observed, not injected (2026-10-19 09:05Z)

Four digest runs that had been CANCELLED on 2026-10-18 executed at **09:00:01.9–09:00:02.5Z today** and each
posted a 94-byte digest to the forum (`channel.posted` ×4, `run=r000000004129/4161/4193/4225`, destination
`1558589728671146105`; all four now `succeeded`, `dur_ms` 1.9–2.5 s, measured with `pii run ls` at 09:05Z).
`run.cancelled` ×4 was in the log from the day before, and the only suspended run is r000000004353 (12:00Z).

Mechanism, read in the tree: `cancel_run` (`crates/pii-server/src/engine/runs.rs:306-347`) closes the run and
emits `run.cancelled` but never touches the timer wheel; `pii_store::timer::TimerWheel` has `put_timer` /
`next_timer` / `take_due` and **no removal**; `scheduler.rs:87-91` takes the due entry and calls
`spawn_resume(...)` with `let _ =`, so a resume of a terminal run is neither refused nor reported. A cancel is
therefore only a list operation: the schedule survives it, and at the instant it fires the run again.

Two consequences worth separating. (1) **The user's forum received four duplicate digests this morning** — the
four posts are the evidence, and nothing can unpost them. (2) **`since_deploy` reads `posted: 0`** for this
event: those runs predate spec 188 and carry an empty revision, so they land in `unattributed` (1,850) and
the store-only counter cannot see their posts. A zero `posted` with `unattributed > 0` means "no post by
THIS revision", not "no post left the host" — the operator must read both numbers together.

Filed as **spec 196 / row A63** (`a cancelled run's timer does not fire it`): the wheel gains `remove_run`,
`cancel_run` disarms and reports `disarmed=<n>`, and a resume of a terminal run is refused with a named
`run.resume.refused` event instead of a swallowed `let _ =`. The 12:00Z run (r4353) is untouched and is the
intended occurrence observed separately (`_tmp/occ-watch/`).

E1's latency figure is corrected above (31.7 s from the log, not ≤14 s). E8 re-verified by reading the
artifacts (`put.json` id == the bm25+vector hit, `degraded:false`). 187/E2/E3/E9/E10/E12 scripts were checked
byte-identical to `d0858a6`, so their recorded outputs belong to the committed harnesses, not to drift.

## The digest timing correction (measured, then fixed)

The standing state said the digest was armed for 09:00 LOCAL = 12:00 UTC. The store journal proved
otherwise: the wheel entry for run r000000004129 carried `timer:…:tick:1792400400000` = **09:00:00Z**
— the GRAPH's timer-node cron (`0 9 * * *`, UTC) is the knob; `[[cron]] schedule` in pii.toml is a
display mirror (graph comment line 13 already said so). The config's `schedule = "0 12 * * *"` had
been recording the intent while the graph fired six hours early. Fix: `graphs/discord-digest.toml`
timer node now `cron = "0 12 * * *"` (with the reason in a comment), installed copy updated, daemon
restarted, the stale 09:00Z armed run cancelled (`r000000004129 --reason "stale duplicate"`), and the
fresh armed run r000000004353 verified in the wheel for `1792411200000` = 12:00:00Z. Final pair:
cron_entries = 1, suspended = 1 (r000000004353 only). Committed as d0858a6.

## The 09:00Z finding's fix, exercised against the deployed binary (10:05Z)

Spec 196 (`5a6445e`) removes the run's wheel entry when the run is cancelled AND refuses a resume of a
finished run loudly. `scripts/exercise-196-past-the-instant.sh` replays the production shape from scratch:
arm one timer run a few minutes ahead, cancel it, drop the `[[cron]]` from the config so nothing can re-arm,
restart, and watch the clock.

| binary | runs that executed at the instant | posts |
|---|---|---|
| deployed `121fb6de67e1` (pre-fix) | 2 | 2 |
| this tree, post-196 | 0 | 0 |

So the defect the live daemon produced at 09:00Z is reproducible on a scratch store with the deployed
binary, and absent with the fix. Spec 193 (`65588c3`) was exercised on the same kind of scratch daemon: the
log carried exactly one `cron.fired` naming the cron entry, the graph and the armed run.

Two harness lessons from this pass, both mine, both cost time:
- An exercise binary must be rebuilt AFTER the commit it is meant to test. Twice tonight `target/release/pii`
  was older than the fix under test, and the "failure" it reported was pre-fix behaviour — the check
  disagreed with production because the check was reading the wrong artefact.
- A plan row's `paths` column must name every path a change can ripple into (test module beside the file,
  the other crate's tests). Two rows committed green gates over dirty trees whose HEAD alone would have been
  red (`46fa5830` is the follow-up commit that put it right). open-items 9a, measured twice.

## 070 §1, its seven steps, as of 2026-10-19 10:30Z

| # | step | tonight's reading | verdict |
|---|---|---|---|
| 1 | one cron entry, one armed run | `suspended` = 1 = `cron_entries` after a `kill -9` restart (E1) and after every change tonight; spec 187's scratch proof stands | **PASS** |
| 2 | the occurrence itself: exactly one digest | the 09:00Z slot fired FOUR runs — all four had been cancelled on 10-18 (spec 196). The graph now fires at 12:00Z; the occurrence is being watched live (`scripts/checks/digest-occurrence-1200Z.sh`, evidence below) | **PENDING — measured at 12:16Z** |
| 3 | production stops being the test bench | 13 `daemon.started` in 24 h vs 2 installs (was 85 vs 6). Two of the thirteen are tonight's deliberate `kill -9` (E1) and its restart | **RED (improved)** |
| 4 | a recorded full gate on the deployed revision, from a clean export | `cargo run -q -p xtask -- fresh-gate` on HEAD `9045484` → `PASS: build`, `quality-gate: ALL PASS`, `fresh-gate: PASS`, export `git archive`d from HEAD (deleted by the tool on success). Caveat: the shared target dir was warm, so the build line is not a from-scratch compile proof | **PASS (with that caveat)** |
| 5 | a card for every install | 11 aside dirs (`/root/.local/bin.old-*`, `pii.old-*`) vs 3 cards on disk | **RED** |
| 6 | the soak: 7 days from the store | not started; the earliest honest start is after a deploy carrying 193 (the occurrence witness) and 196 | **NOT STARTED** |
| 7 | the `pii-mcp` swap | `pii-mcp.new` staged, not swapped; no probe data tonight | **UNCHANGED** |

**The verdict is still not earned, and cannot be tonight**: step 2 closes at 12:16Z at the earliest, and step 6
is seven days from whenever step 2 first passes on a deployed binary that carries the fixes.

## E2 and E3 re-run independently (10:36Z, scripts/exercise-e2-e3-graphs.sh)

Both rows above were the earlier pass's ad-hoc runs; this pass re-ran them as one script, on two scratch
daemons, and printed the event lines:

- **E3 PASS** — `{"kind":"cron.start.refused","fields":{"cron":"typo","graph":"no-such-graph",
  "error":"not found: graph 'no-such-graph'"}}`, the daemon stayed healthy, and the refusal named the graph.
- **E2 PASS** — `{"kind":"daemon.graphs_fallback","fields":{"looked_in":"…/run/graphs",
  "using":"…/state/graphs"}}`, the daemon stayed healthy, and a cli-channel message still started a run
  through the fallen-back directory (runs=1). First attempt at this failed for a harness reason worth
  recording: the graph must EXIST in the fallback for "runs continue" to mean anything — a name that exists
  nowhere is refused, correctly, and proves nothing about the fallback.

## The 12:00Z occurrence — scored (12:16Z)

One transition out of suspended (`r000000004353`), one `channel.posted` (94 bytes, 12:00:02.007Z), no second
run at +6 or +16 min, re-armed to 1 (`r000000004356`), `posted` 0→1, `post_refused` +0. **Step 2 of 070 §1
PASSES**: the first occurrence on the corrected 12:00Z knob produced exactly one digest. The store journal's
`Finished` entry carries `terminal: Success` and the wheel holds the next tick at `1792497600000` =
**12:00:00Z 2026-10-20**, exactly +24 h.

## The 15:00 pass — the contradiction resolved by measurement, and every exercise re-run fresh (2026-10-19 12:45–12:55Z)

Context at pickup: the overnight pass had already (a) measured the deployed binary stale and swapped in
`4f13db5a…` (revision stamp `121fb6de67e1`, built from HEAD at the time), (b) run E1–E12, and (c) scored the
12:00Z occurrence. This pass answers the remaining standing-state contradiction, re-runs the live-daemon
exercises against the now-current tree, and adds the two measurements the standing state asked for.

### The "diagnostic kind" contradiction — resolved: the diagnostic never shipped

The standing state said a live run prints `unavailable: net: read: peer closed connection without sending
TLS close_notify` while a diagnostic compiled into the binary ("string 'diagnostic kind'",
`crates/pii-net/src/service/conn.rs`) never appears. Measured, not assumed, three ways:

1. **The live process never carried the error text since its start.** The daemon serving this pass started
   07:14Z (pid 2665559) and its log contains **zero** occurrences of `net: read: peer closed`. The
   last such error in the whole log is `ts_ms 1792394069` (04:54:29Z) — the pre-deploy binary
   (4182856b…), which is exactly what spawned the contradiction.
2. **The diagnostic string never existed in ANY shipped artifact.** `grep -ac "diagnostic kind"` on the
   previous deployed binary (`pii.old-1792393586`) = 0; on the current binary = 0.
   `git log --all -S"diagnostic kind" -- crates/pii-net/src/service/conn.rs` = no commit. The string
   appears only in a code COMMENT (conn.rs:192) describing an already-removed diagnostic.
3. **The 04:54Z error text came from the pre-fix read arm**: revision `4182856b`'s conn.rs read is an
   unconditional `.map_err(|e| …"net: read: {e}")` — no `UnexpectedEof`/`close_notify` arm, no diagnostic
   print. The premise "diagnostic compiled into the binary" was stale handoff drift, measured false.

**Therefore: nothing left to patch; the fix was already deployed.** Requirement: "Measure first, patch
second" — measured; the site the standing state pointed at does not fire because it does not exist.

### Task 2 — rebuild with proof of compile, and the PONG bar re-met on a fresh run

`cargo clean -p pii-net -p pii-cli -p pii-server` then rebuild: `Compiling pii-net v0.1.0`, `Compiling
pii-server`, `Compiling pii-cli`, `Finished release` rc=0. Deterministic: force-`touch` + rebuild produces
the byte-identical sha256 `fd5753e3d7114c05fe22ecbf43c18b1558fa5743c3b80508a4d691621b5467cc`. Revision
stamp in the binary `PII_TRAJECTORY_DIRf6b1e18c74a6c`; `"diagnostic kind"` occurrences in the binary: **0**
(removed; task 2's "confirm removal" is satisfied by the source diff + the binary grep).

`pii -p "Reply with exactly: PONG-GLM" --model openai:glm-5.3-flash` (pii.env exported) → run **r000000004357**,
rc=0, ~1.4 s: full "PONG-GLM" (deltas "P" + "ONG-GLM"), `"net: read"` in node_finished: **0**,
`"close_notify"`: 0, `run_finished` = `terminal.kind=success` with **no `output.error` field**.
All three bar conditions: MET on the fd5753e3 build.

Note on `pii health`: it printed `0.1.0 (121fb6de67e1)` — that is the DEPLOYED binary's revision. The deployed
binary carries the conn.rs close_notify fix (spec 191) and the deployed PONG (run r000000004356) already
passed the bar (overnight results); the fd5753e3 tree-only build re-proves the bar with today's tree.
Swapping the deployed binary again tonight would need its own card (spec 197's rule) and is deferred to
the operator.

### Task 3 — the queue is EMPTY

`state.py pick` → `NONE`. Row statuses (state.py list): A55 done (1e0883c), A56 done (97a6cdb), A57 done
(64f4d95), A58 done (1c41bd1), A63 done (5a6445e), A64 done (6aad6d8), A65 retired. **No open work.**

### Exercise verdicts, all re-measured 12:49–12:55Z

| id | injected | pii did by itself | verdict | evidence |
|----|----------|-------------------|---------|----------|
| E1 | `kill -9` the LIVE daemon (pid 2665559, literal argv-verified pid, 12:49:09Z) | systemd returned it: new pid 2871308, `daemon.started` logged ts 12:49:16Z (≈7.3 s return), suspended list before/after byte-identical (r000000004356) | PASS | `/projects/_tmp/e1-before.json` == `/projects/_tmp/e1-after.json`; pii.log `daemon.started pid 2871308` |
| 187 | four hard restarts on a SCRATCH daemon (`scripts/exercise-187-restarts.sh`, re-run) | armed held at 1 across starts 1–4; verdict line "187 HOLDS — four starts, one armed run" | PASS | workspace `/projects/_tmp/exercise187-dxeDld` |
| E2 | graphs dir missing on a scratch daemon (script re-run) | `daemon.graphs_fallback{"looked_in":…/run/graphs,"using":…/state/graphs}`; a run still started | PASS | script output 2026-10-19 12:50Z |
| E3 | broken cron graph on a scratch daemon | `cron.start.refused{"cron":"typo","graph":"no-such-graph","error":"not found: graph 'no-such-graph'"}` | PASS | same output |
| E8 | `pii memory put "ember-vellum-morrow-…"` on the LIVE daemon, then search | hit on lanes `bm25+vector`, `degraded:false`; doc id eca0d69f31587375789bfc939f5eda50 | PASS | `/projects/_tmp/e8-put.json`, `/projects/_tmp/e8-search.json` |
| E6/E5 record | none injected; log read AFTER the fixes | poll.failed total 6,825 by 12:55Z: 6,817 pre-deploy (last read-timeout 11:24:08Z) + **8 since the 07:07Z deploy** — 1 read-timeout (os error 11, 11:24:08Z) and **7 Discord-429 polls in one ~28 s burst at 12:48:06–12:48:34Z** (right before my E1 kill) with NO retry_after token in the text (the deployed binary predates 194), which then stopped on its own. Discord 429 mentions: 56 pre-deploy + 7 in that burst. retry_after complaints: 3,672 pre-deploy, 0 since. egress.denied: 6,656 historical, last 10-17 03:46Z, none since either fix. So: the deployed poll cadence still re-polls a 429 at ~4–7 s (pre-194 behaviour, LIVE, not historical) and the burst self-heals only because the 429 clears; 194/195 are in the tree and NOT deployed — the next deploy card carries them | **E5/E6 behaviour: LIVE (pre-fix cadence) but quiet-scale** — one 7-poll burst per ~5 h, self-extinguishing; all the 4-figure burdens are historical | numbers above read from pii.log by timestamp filter |

**Pair check** (task 5, after every change): `run ls --status suspended` length = 1 = cron_entries
(r000000004356, armed for 12:00:00Z 2026-10-20 in the wheel — verified present, `tick:1792497600000`),
after build, PONG run, E1 kill/restart, E8, and the E2/E3 scratch runs. No duplicate ever appeared;
no cancel was needed.

## 14:30–14:40Z — the deploy landed, and the bar was re-met on the DEPLOYED artifact

The orchestrator's installer (card `_tmp/deploy/card-420fe8a7e946.json`) built from a clean `git archive
HEAD` export and installed revision **`420fe8a7e946`** (sha256 `b4deb82dcce91a46…`), carrying specs
193/194/195/196. The daemon restarted 14:34:08Z and posted its verification line naming that revision.

- **A check of mine raced the install and read a false state.** At 14:33–14:34Z `health --json` said
  `cron_entries: 0` and `cron ls` = `[]` while the installer was still writing the config (config mtime
  14:34, daemon start 14:33:36 — the daemon had started from the pre-install file). After the unit's own
  restart, the same commands read `cron ls` = `[digest, 0 12 * * *]`, `cron_entries: 1`. Recorded because
  it is exactly the "suspect the check first" case: the config was never missing its cron table.
- The installer rewrites `[core] model` to `llamacpp-server:granite-4.2-3b` from the production example
  (`deploy/install-pii.sh:118`) — so the deployed default model changed from the hand-edited
  `openai:glm-5.3-flash`. The digest graph is timer → recall ×2 → post (no model node), so the digest is
  unaffected, and an explicit `--model` still wins (used below). Flagged for the operator, not hidden.
- **PONG bar, re-met on the deployed binary:** `pii -p "Reply with exactly: PONG-GLM" --model
  openai:glm-5.3-flash` (pii.env exported) → run **r000000004417**, rc 0, 2.43 s, complete `PONG-GLM`
  (`run get` status `succeeded`), `node_finished` carries no `net: read`, `run_finished` is
  `terminal.kind=success` with **no `output.error` field**. Log lines containing `net: read` since the
  14:34:08Z start: **0**. `grep -ac "diagnostic kind"` on the deployed binary: **0**.
- **Pair after the change:** `run ls --status suspended` = 1 (r000000004356) = `cron_entries` 1.
- **E5/E6 status after the deploy:** the deployed binary now carries 194/195, and both fixes were measured
  on it in a two-arm A/B against the previous binary (rows below): E5 72→4 polls with `retry_after` in the
  text, E6 two paced refusals then one post where the old binary double-dialled. The remaining live
  observation is a REAL Discord 429 window — none has occurred since 14:34Z, so that line is still owed to
  the log; it is no longer owed to the code.

### Exercises re-run on the deployed revision (14:36–14:45Z)

The deploy replaced the artifact, and spec 196 touched the timer wheel, so the wheel's two directions were
measured again on `/root/.local/bin/pii` (revision `420fe8a7e946`), each on its own scratch daemon:

| id | injected | pii did by itself | verdict | evidence |
|----|----------|-------------------|---------|----------|
| E8 | `memory put` of a fresh nonce `newton-143612-driftwood`, then `memory search` | doc `651e5a5ab35bde9ce81b47befff93641` is the top hit, lanes `bm25+vector`, `degraded:false` | PASS | `_tmp/e8-deployed.json` |
| 187 | four hard restarts on a scratch daemon | armed stayed 1 at every start; verdict line printed | PASS | `/projects/_tmp/exercise187-WsUdG8` |
| 196 | arm a timer, cancel it, remove the `[[cron]]`, wait past the instant | **0 executed**, 2 failed-without-executing, **0 posts** — nothing fired | PASS | `/projects/_tmp/ex196-Q8WfdN` |
| E10 | the timer node's cron set two minutes ahead | one terminal run + exactly one re-arm at `+86400000 ms` (1792420920000 → 1792507320000) | PASS | `/projects/_tmp/ex19e10-pZAGWI` |
| E5 | a fake Discord answering every poll 429 `retry_after: 2`; one harness, two binary arms | deployed `420fe8a7e946`: **4 polls in 7 s**, gaps 2005/2081/2006 ms, error text carries Discord's body; old `121fb6d`: **72 polls**, gaps 103–106 ms, text `poll status 429` only | PASS | `/projects/_tmp/ex194-WCbcV1` (new), `ex194-osSL1W` (control) |
| E6 | a fake Discord refusing the first dial with `retry_after`; same two arms | deployed: two `post_refused` lines `paced by the provider for 2480 ms` / `1875 ms`, then ONE `channel.posted`; old: run 2 dialled again inside the window (2→3), nothing paced | PASS | `/projects/_tmp/ex195-aHuCEE` (new), `ex195-Nx84d5` (control) |

E10's terminal run is `failed` by the harness's design: its scratch channel points at the nonce destination
`"scratch"` with no token, so the post cannot land; the wheel, not the post, is what that exercise asserts.
E1 was deliberately NOT re-killed after the deploy — the returning mechanism is systemd and the binary does
not carry it, and a third kill today would distort 070 §3's start count; today's 12:49Z E1 measurement stands
as the proof.

Still not "production ready": the soak window (070 §1 step 6) can only start at this deploy — 14:34Z,
2026-10-19 — because it is the first binary that carries the occurrence witness (spec 193); its first counted
occurrence is 2026-10-20 12:00Z, so the earliest honest closing date is **2026-10-27 12:00Z**. Step 3's
stop-being-the-test-bench tally and step 5's card-vs-aside-dir count need the same window to mean anything.