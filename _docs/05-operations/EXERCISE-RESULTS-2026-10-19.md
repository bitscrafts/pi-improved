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
| E6 | none (symptom already live: forum post 429) | outbound records `channel.send.failed` + `channel.post_refused` with Discord's `retry_after:132.176` IN THE ERROR TEXT, 0 since the deploy — but only `reply()`'s blind one-retry exists (`router.rs:171`); nothing READS `retry_after` to pace the sender, so "backoff respects retry_after" is not met by construction | **FAIL (historical ×1836, mechanism still unbuilt)** | 1836 × send.failed + 1836 × post_refused, last 10-18 09:02:49Z; `retry_after` in `crates/`: error text only |
| E7 | none observed live this pass | denial is per-connection in the shipping code (grant window 16 MiB in/out per connection, `crates/pii-server/src/daemon/config.rs:698`); committed tests prove an oversized response denied while later posts succeed, and that concurrent connections cannot spend each other's window — 6/6 green on the deployed tree. Historical `channel.egress.denied` (57,284, last 10-17 03:46Z) predates the per-connection fix | PASS (tests) / historical burden cleared | `cargo test -p pii-net --test budget_survives_a_day --test budget_window`: `ok. 4 passed` + `ok. 2 passed` |

Honest residue:
- The stray doc `1299bfbf0ab382c160af572b95cae4f2` (text "--help") was written into the live memory
  store by an accidental `memory put --help` (positional arg) this morning; no delete verb exists.
  Left as-is and disclosed here rather than hidden.
- E5/E6 remain the two OPEN defects: one incident per poll-outage (E5's incident half) and
  reading `retry_after` for real pace (E6). Paths: `crates/pii-channels/src/discord.rs`,
  `crates/pii-channels/src/router` (poll loop), incident scan (`pii incidents scan --source runtime`).

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