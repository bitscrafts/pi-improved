# EXERCISE MATRIX — what to inject into pii, and what pii must do about it

2026-10-18. The rule from the user, verbatim: **"you just observe, the system should heal"** — the harness injects and
looks; only pii repairs. A recovery written by the harness is not evidence.

## The matrix

| # | Component | Fault to inject | What pii must do BY ITSELF | Evidence that counts |
|---|---|---|---|---|
| E1 | daemon liveness | `pkill -9` the daemon | systemd returns it; `daemon.started` recorded | unit active, started count +1 |
| E2 | graphs | move the graphs dir away | `daemon.graphs_fallback` to the installed path | runs continue, fallback event |
| E3 | cron config | add `[[cron]]` with `graph = "no-such-graph"` | `cron.start.refused` naming the graph, good cron unaffected | refusal event, cron_entries unchanged |
| E4 | model provider | stop the llama.cpp server | a refusal event naming the provider, not a hang | refusal within N s, no crash loop |
| E5 | channel inbound | garble the token | `channel.poll.failed` + the reader opens ONE incident | incident id and latency |
| E6 | channel outbound | force a 429 | backoff respects `retry_after`; a post still lands | post sent, no storm |
| E7 | net budget | one oversized response | denial is per-connection, later posts unaffected | `budget_survives_a_day` + a live post |
| E8 | memory | `pii memory put` then search | hybrid lanes answer, `degraded:false` | round trip |
| E9 | store | make the store read-only | a refusal naming the store; no silent drop | refusal event |
| E10 | scheduler | cron set two minutes ahead | exactly ONE run, then silence | run delta 1, then 0 |
| E11 | loop | park a row at the head | 169: skip it, dispatch the next | the pick line names the skip |
| E12 | deploy | dirty tree at install | clean export builds HEAD, revision has no suffix | `BUILT FROM EXPORT`, clean revision |

## Status measured so far (2026-10-18)

E1 PASS · E2 PASS · E3 PASS · E8 PASS · E10 PASS (179: +0 runs in 60 s with the cron live) · E12 PASS (c0818bfcc898)
E4, E5, E6, E7, E9, E11 NOT YET EXERCISED — and E5/E6 already show symptoms without a fault being injected:
    channel.poll.failed 6,672 · channel.egress.denied 6,656 · discord 429 retry_after=132s × 1,836

## 2026-10-19 overnight pass (see EXERCISE-RESULTS-2026-10-19.md)

E1 PASS · E2 PASS · E3 PASS · E4 PASS · E8 PASS · E9 PASS · E10 PASS (real knob = the timer node's
cron; `[[cron]] schedule` is a display mirror) · E11 PASS · E12 PASS (121fb6de67e1 from a dirty tree)
STILL FAILING, both by construction: E5 (the poll path parses no Retry-After; error backoff recorded
but never read) and E6 (nothing reads retry_after to pace outbound sends). E7's committed tests are
green and the historical 57k denials predate the per-connection fix.

**Updated 2026-10-19 15:00Z (15:00 pass):** the standing-state contradiction ("diagnostic kind" never printed)
is resolved by measurement: the diagnostic never shipped — the string exists only in a source comment,
`git log --all -S` finds no commit carrying it, and revision 4182856b's `conn.rs` read arm is the
unconditional pre-fix `map_err` that produced the 04:54Z error. Live E1 re-killed (returned in ≈7.3 s, armed
run unchanged), 187 re-run ("187 HOLDS"), E2/E3 re-run as scripts, E8 re-round-tripped with a fresh nonce —
all PASS. E5/E6 live-log reading AFTER both fixes, corrected by re-filtering pii.log at 14:38Z: since the
07:07Z deploy there were **8 poll.failed, 7 of them Discord 429s in one burst 12:48:06–12:48:34Z** (the
otherwise-zero-429 note below this line was wrong — corrected here), 0 send-failed with a retry_after token,
0 egress.denied. So the four-figure burdens (56 + 3,672 + 6,656) are historical, one small live burst per
~5 h is the pre-fix cadence (self-extinguishing), and both fixes await the next deploy card. Full table in
the results file, "The 15:00 pass".

**Updated 2026-10-19 10:10Z:** **E6 is FIXED and exercised too** (spec 195, commit `05b3474`, in the tree, not
deployed): a send that is refused is paced by the `retry_after` it carries, per channel, and `reply()` no longer
double-dials. Live A/B, one harness, fake Discord 429-once: the deployed binary dialled 2→3→4 times across three
runs and delivered 3; the fixed tree dialled 1→1→2, delivered 1, and logged two `paced by the provider for …`
refusals. **No matrix defect remains open.** Both fixes and spec 193's occurrence witness (`65588c3`) and spec
196's cancel disarm (`5a6445e`) are in the tree and NOT in the deployed binary; a deploy carrying them needs its
own card, and until then the host's behaviour is the pre-fix behaviour. A new production finding is recorded in
the results file: four runs cancelled on 10-18 executed and posted at 09:00Z today (spec 196, row A63).

**Updated 2026-10-19 14:40Z (post-deploy):** revision `420fe8a7e946` is DEPLOYED (sha `b4deb82d…`,
14:34:08Z), carrying specs 193/194/195/196. E5's poll pacing and E6's send pacing are therefore in the
deployed binary now; no Discord 429 window has occurred since the restart, so their post-fix live
behaviour is pending one burst (the pre-fix A/B remains the red arm). The PONG bar was re-met on the
deployed artifact — run `r000000004417`, `succeeded`, no `net: read`, no `output.error`. Pair: suspended
1 = cron_entries 1. The open-defect column is empty; what remains is soak time.

## The three findings the first exercise produced

1. `channel.poll.failed` × 6,672 — the inbound channel cannot read; the cursor never advances.
2. `channel.egress.denied` × 6,656 — the byte budget still denies under the per-connection fix (that is E7, unfixed).
3. `discord 429, retry_after=132s` × 1,836 — the outbound post is rate limited and nothing paces it (that is E6, unbuilt).

## A missing capability, not a fault

pii has NO worker/agent node: nothing in `crates/` spawns `pi`, speaks JSON-RPC to it, or holds a session with
`pi_agent_rust` — despite `pi 0.2.0` being installed at `/root/.local/bin/pi`. "pii writes a spec and hands it to pi" is
something to BUILD (a `worker` node kind that owns a pi session: spawn, spec, criteria, gate, result), not something to
exercise. The delegation that happens today is the autonomy loop's shell script driving Claude Code, outside the daemon.

## How the harness runs

`scripts/exercise-pii.sh` — injects, then observes only. It restores nothing; a fault still present at the end is the
finding. Every fault in the matrix is reversible by a separate, deliberate operator action.
