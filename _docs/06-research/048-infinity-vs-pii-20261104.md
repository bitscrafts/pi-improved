# Infinity (AWS/Hydro) against pii — what fits, what does not

Analysed 2026-11-04 from the primary source (`github.com/hydro-project/infinity`, Apache-2.0, Rust,
read same day) plus two the workspace memory entries (`research/aws-infinity-agent-runtime-rust`,
`insights/aws-infinity-agent-runtime-rust`).

**Correction to the the workspace memory entry:** it records 687 stars; the repository shows **434** today. The entry's
figure is wrong or was mis-read. Everything else in it checked out, and the repo adds one name the
entry omits: the tool protocol is **RAP — Reactive Agent Protocol**, with a compatibility layer that
runs existing MCP servers unchanged.

## What Infinity is, in one paragraph

An agent is a stackless coroutine. Its conversation history IS its continuation, so at every tool call
the runtime persists the history and discards all in-memory state. Work happens in **slices**: load
history → one model completion → persist → dispatch **at most one** tool → exit. Any input (user text,
tool result, timer, event) arrives as a message on the thread's own queue. An idle agent is pure data:
no task, no stack, no connection — ~100 KB, which is why 75k agents fit in 8 GB.

## What pii already has, so this is not a rewrite

```
durable steps        the run journal + resume-by-replay (SNAPSHOT_EVERY = 32)
durable suspension   `timer` and `gate` suspend the run and survive a restart
effect discipline    `is_effect` per node kind, and a replay refuses an effect node — pii's
                     exactly-once is enforced at the ENGINE, which Infinity does NOT have
                     (their answer is per-tool dedup state in DynamoDB)
ordering             one slice per run, guaranteed by the store's single writer
```

That last line matters for the comparison: **pii is ahead of Infinity on side-effect safety and behind
it on mid-turn resumability.**

## The four gaps Infinity names for pii

**G1 — a coder turn has no checkpoint.** pii's `coder`/`agent` node spawns an external agent and waits
for the whole turn. If the daemon dies mid-turn, the turn is lost — the exact failure already paid for
in this workspace (a gateway restart killed a paid worker mid-request; hard rule 0f). Infinity's slice
model would make the turn resumable **per tool call**: persist after each completion, dispatch one
tool, exit. In pii this is not a new engine — it is the journal it already has, applied one level
deeper, inside a node.

**G2 — `tool` is synchronous.** pii's `tool` runs a process and blocks the slice. Infinity's tools
*start* work and return; the result arrives as a message. For pii that means a node that can await a
long command (a 3-day CI, a build) with **no live run and no blocked worker** — pii's `wait`/`gate`
already prove the suspension mechanism, so the gap is the result path, not the mechanism.

**G3 — `spawn` has no return half.** pii has `call` (inline, waits) and `spawn` (child run, returns an
id, never reports). Infinity's `spawn_thread` plus `report_to_parent` is fire-and-join-later: the child
reports arrive as events on the parent's thread. pii can express "start it" but not "hear back".

**G4 — an external event cannot reach an agent mid-turn.** Infinity injects a synthetic assistant tool
call the model never made (`receive_event__injected`) carrying `original_tool_name`,
`original_tool_call_id` and `original_args`, followed by the tool result. That preserves the
one-call-one-result invariant and the ordering. pii's events (cron fires, MQTT messages — MQTT is in
the production path) currently start a new run or are invisible to a running agent.

## What pii should NOT take

```
their AWS reference stack   SQS FIFO + Lambda + Aurora DSQL + DynamoDB + EventBridge. pii is ONE
                            binary with a local store; adopting that would delete the product's
                            premise
their concurrency model     actor-per-thread. pii's unit is a run over a graph; the scheduler already
                            gives per-run ordering, and an actor layer would fight the engine
their crate as a dependency 434 stars, early research release, protocol (RAP) still moving. Borrow the
                            SHAPE — results arrive as messages, history is the continuation — not the
                            dependency. pii must keep running standalone
```

## Five proposals for pii, ranked by what they fix

```
P1  A CODER TURN IS SLICES           persist the turn's history after every completion, dispatch one
    (fixes G1)                       tool, resume from the journal. The turn survives a restart, and a
                                     killed worker costs one tool call rather than the whole night.
    why first                        it is the failure this project has already paid for, and pii's
                                     journal already exists — this is depth, not new machinery.

P2  AN ASYNCHRONOUS TOOL             `tool.async = true` returns a handle; the result is delivered as
    (fixes G2)                       an event that resumes the run. Proof: a graph that starts a long
                                     command and a run that is NOT live while it runs.

P3  SPAWN REPORTS                    `spawn` gains a `report` port: the child's verdict arrives on the
    (fixes G3)                       parent's port as an event. `night`'s review step is the first
                                     real user — it wants the child's report, not its exit code.

P4  EVENTS WITH PROVENANCE           an MQTT/cron/Discord message becomes a synthetic tool result with
    (fixes G4)                       the originating tool named, so an agent mid-turn receives it
                                     without a second run and without breaking ordering.

P5  MEASURE HIBERNATION              publish pii's own number for what a suspended run costs. Infinity's
                                     claim (100 KB/idle agent, 75k in 8 GB) is the same property pii's
                                     `timer`/`gate` suspension already has; pii has never measured it,
                                     and a number nobody measured is a number nobody can defend.
```

## The one-line verdict

**Infinity validates pii's direction and points at the place pii is still synchronous: inside a node.**
pii's engine is more conservative where it matters (effects and replay), and less capable where the
work is long (agent turns, tools, child reports, external events). P1 is the row I would take first,
because it converts an existing guarantee — the journal — into a property the product can advertise:
*a killed host costs one tool call.*
