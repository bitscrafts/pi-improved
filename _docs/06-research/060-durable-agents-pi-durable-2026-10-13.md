# 060 — Durable agents (Pi Durable) and what pii should take from them

**Source of the finding.** A user drop, 2026-10-13: <https://youtu.be/DNDYYHsypZg> — *"Pi Durable PROACTIVE
Agents: Jev Triggers My ON-CALL Pi Agent"*, IndyDevDan, 2026-10-12, 35:16, 31.5k views. Its description links
the codebase (<https://github.com/disler/durable-pi>) and the primary source (<https://earendil.com/posts/pi-durable/>).

**Primary source first, because the video is a walkthrough of it.** Earendil shipped **Pi 1.0** and, beside it,
**Pi Durable** — "an experimental new package … for long-running, durable, and malleable agents that can run
anywhere" (`earendil.com/posts/pi-durable/`, fetched live 2026-10-13). The codebase the video demonstrates is
`disler/durable-pi`: **MIT**, TypeScript, created 2026-10-05, pushed 2026-10-11, 89 stars, default branch `main`
(GitHub API). Its description: "Durable Pi playground and agent network: persistent, multi-device durable
agents with web and terminal clients".

## What the approach actually is

Three ideas, each with a mechanism behind it.

1. **A run survives its own process.** In Pi Durable *every step of a run is a task that stores a checkpoint
   before it moves on*. A new process opens the same storage, finds the unfinished tasks and continues each
   from its last checkpoint. A model request cut off by a crash is **re-sent**, and the partial answer stays in
   the transcript marked **aborted**. A tool call cut off by a crash **reruns only if it declares
   `replay: "safe"`**; otherwise the model is told it was **interrupted**, with the output stored so far, and
   decides what to do. Queued messages stay queued, and a `requestId` makes a submission **exactly-once** — a
   client that retries after a crash gets the original answer back instead of asking twice.
2. **The device owns the run; clients only watch and steer.** `dpi-backend` runs per device (port 4780, a
   launchd/systemd service) and owns the harnesses, `sessions/<id>/agent.sqlite` and the device's shared App
   State. A web hub, `dpi-tui` and a single `curl` are thin clients over one v1 HTTP API with a bearer token;
   an address names the whole thing (`mac-mini:api-refactor:fork` = device : session : agent). Close the
   browser, quit the terminal or restart the machine mid-tool, and the run picks up from its last checkpoint.
3. **Proactive: work starts before a person arrives.** The demo (`apps/proactive-dpi-demo`) tails a mock Cloud
   Run log stream, sends **one bounded judgment call per row** to a classifier (Jev: `normal` / `suspicious` /
   `incident` + an impact score, a few hundred ms, ~**$0.00003 per row**), and **code owns the thresholds**: three
   incident verdicts sharing one fingerprint within 120 s open an incident and start a durable agent on its
   own. The agent investigates, finds the root cause, writes the fix and a regression test that it proves fails
   on `main` and passes on its branch, then calls `deploy` — which **waits for approval**. Measured timeline:
   bug at T+5.0 s, incident at T+5.8 s, root cause ~T+18 s, ready-for-review T+21 s, approval asked T+25.3 s.
   An incident costs about **$0.09** of Sonnet 5.5; the deploy is a durable task with phases
   `approval → rollout → verify`, so a restart resumes the phase and nothing runs twice.

The layering is the load-bearing part, and it is stated as a table in the demo README:

| layer | decides | in the demo |
|---|---|---|
| **code** | thresholds and side effects | 3 incident verdicts with one fingerprint within 120 s open an incident |
| **a small model** | bounded judgments, one call per row | `normal` / `suspicious` / `incident` + impact |
| **the agent** | open-ended work | investigate, fix, prove, deploy — then wait |

Their own axiom, worth measuring yourself: *your agent should be able to understand its own harness* — which
they size as **~15,000 lines of source, ≈150k tokens with GPT and ≈250k with Claude**.

## What pii already has (so the comparison is honest)

| Pi Durable | pii today |
|---|---|
| one process owns a storage; clients attach | one daemon owns the store (`redb`), channels are clients (`cli`, `discord`, `telegram`, `webhook`) — 059 measured this as "the device owns the run" |
| bearer token per device | **capability scopes** per graph/node/principal, plus a connect-time grant store (`pii-net`) — strictly stronger than a bearer token |
| `requestId` exactly-once submissions | idempotency keys on channel posts (128) and on promotion decisions (131) — present, not universal |
| durable tasks with phases + approval | `approval.redb` + the promotion quarantine/revert gate (`pii learn`) — the human-near-the-loop gate exists |
| proactive trigger from a real event | `[[cron]]` graphs and channel pollers exist (`discord-triage`, `discord-digest`) — the surface is there, nothing is deployed |
| extensions = prompt sections + tools + hooks | node kinds, graphs, tiers, and the node's capability scope — the same idea in Rust |

So the gap is not the architecture. It is **the durability contract**: pii persists run *records*, Pi Durable
persists *the step*, and the difference shows exactly where pii was measured to hurt.

## The five things worth taking, ranked

1. **Run recovery with replay semantics (the strongest).** pii's runs are in the store, but a run whose
   *process* died mid-node stays `Running` — the daemon has no recovery sweep, and the node's code cannot say
   whether re-running it is safe. Adopt the contract: on daemon start, sweep runs left unfinished; **resume the
   node only if it declares itself replay-safe**, otherwise mark the run `Interrupted` with the partial output
   recorded. This is DO-NOT-REPEAT 1.50's class one level up: 135 made a *panicking node* terminal, this makes
   a *dead daemon* visible.
2. **A proactive judge graph on the local model.** pii now has `llamacpp-server:` (spec 129) and cron graphs
   (128): a `judge` node kind that makes one bounded, schema'd call per row, counters, and a threshold node
   that opens an agent run when N verdicts with one fingerprint land in a window is *exactly* the demo's
   three-layer split — at **zero marginal cost** for the judging layer, where Jev is a paid service.
3. **Durable task phases for anything outward-facing.** `approval → rollout → verify`, each phase
   checkpointed, is what spec 142 (deploy) needs to be more than a script, and what any channel or deploy
   action should have before it is allowed to run unattended.
4. **`requestId` on every ingress, not just the two we have.** Run starts, channel ingress and graph
   submissions should be exactly-once; today only the channel post and the promotion are.
5. **Measure the self-comprehension axiom.** 15k lines is their budget for "an agent can read its own
   harness". pii measured 2026-10-11: **25 crates, 397 `.rs` files, ~89,020 lines of Rust, 1,103 test
   functions** — six times over. The honest response is not to shrink pii to 15k lines, it is to (a) keep the
   entry surface an agent needs small and documented (`pii map`-style), and (b) publish the number rather than
   let someone discover it. A tiny companion: the KPI the video asks you to track —
   *how many agents are doing useful work right now, without you* — is one metric pii does not have.

## What this is not

The repo is a **playground for a week-old experimental package**, not a production system: the network layer
assumes passwordless ssh between devices, the hub has no login, and the demo needs an ElevenLabs key for its
voice. The video is third-party and monetises its own phase-three signup — the technical content is real, the
"future of engineering" framing is marketing. The primitives are what matter, and the primary source states
them plainly.

## Actions

- **Held as RESEARCH, not queued for implementation** (user directive 2026-10-13): **spec 143** (run recovery
  with replay semantics) and **spec 144** (the proactive judge graph). They are marked `STATUS: RESEARCH` in
  their headers and were removed from `scripts/autonomy/plan.tsv`; they go to a claude+opus review first.
- The review's frame is the point the user took from the video: **proactivity applied to pii's own
  self-improvement** — the loop reads its logs by itself, resolves by itself, and asks permission only to put a
  change into production, with the operator as far from the loop as possible and deciding only the end.
- Dual agent-memory entries stored: `workspace/research/pi-durable-durable-agents-2026-10-13` (facts) and
  `workspace/insights/pii-from-pi-durable-2026-10-13` (the angle).

## Update 2026-11-07 — measured against the tree, and a correction

Re-read of the primary source (`earendil.com/posts/pi-durable/`, 2026-10-01) and of Temporal's response
(`temporal.io/blog/the-immortal-life-of-pi…`, 2026-10-08), then each item below was checked in the tree.

**Corrected.** An earlier pass (and the first answer given on 2026-11-07) called Pi Durable's `replay: "safe"` flag
pii's missing piece. That was wrong. pii has the inverse, already enforced:

- `GraphNode::is_effect` / `is_effect_for(cfg)` (`pii-graph/src/node.rs`) is declared by eight node kinds: `hal`, `plan`,
  `spawn`, `queue`, `coder`, `post`, `memory`, `ledger` (the last per configuration).
- The scheduler journals `EffectStarted` with an idempotency key (run, node, input digest) before the effect, and
  `EffectConfirmed` after (`pii-graph/src/effect.rs`, `sched/dispatch.rs`; spec 186).
- Resume of a journal holding an effect started and never confirmed is **refused** (`effect.unknown`); a confirmed effect
  whose `NodeFinished` never landed is completed from the journal, never re-run. `pii run retry --node` is the operator
  accepting the risk. Tests: `effect_not_repeated`, `unconfirmed_effect_refuses`.
- A non-effect node with `NodeStarted` and no terminal event is re-run on resume (`run/resume.rs`, "the documented Phase-1
  boundary"). That IS Pi Durable's `replay: "safe"` behaviour, expressed as "not an effect".
- At boot, runs a dead daemon left `Running` have their agents killed and are closed (spec 242, `engine/orphans.rs`,
  `orphans_are_reaped`); a run whose id was acknowledged is already on disk (spec 305, `acked_run_survives_kill`); a
  `Suspended` run with no timer is re-armed (spec 353, `boot_recovers_suspended`).

So the durability CONTRACT is in place. What differs is a POLICY: pii closes an in-flight run as failed at boot and leaves
resuming to the operator; Pi Durable's opt-in `retry_unfinished` resumes automatically. Pi Durable's own default is the
same as pii's ("mark interrupted, do not retry", `durable-harness.md`). Specs 143 and 144 therefore stay RESEARCH: there is
no measured run that an automatic resume would have saved, and "I would have liked it" is not that measurement.

**Still unmeasured, so not built:** exactly-once run starts. No idempotency key exists on `run start` (searched
`pii-server/src` and `pii-cli/src`); channel posts and promotions have one. Whether a duplicate start has ever happened is
not recorded anywhere. The first step is a counter, not a feature.

**Does not transfer:** the 15,000-line self-comprehension budget (pii: about 90,000 lines in 734 `.rs` files; the useful
equivalent is `graphs/VOCABULARY`, 14 kinds); Temporal's cross-machine failover (pii is one daemon per host by decision);
the package itself (TypeScript; pii is a single Rust binary).
