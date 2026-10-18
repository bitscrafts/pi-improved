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
