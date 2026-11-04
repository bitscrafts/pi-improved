# 049 — Review brief: Infinity vs pii, judged independently, then compared

Read only; do not edit the tree. Answer in English, in the structure at the end.

## What you are reviewing

`_docs/06-research/048-infinity-vs-pii-20261104.md` — my analysis of AWS/Hydro's **Infinity** agent
runtime against **pii** (the project you are in: `/projects/pi-improved`, a single-binary Rust
personal-information server whose work is expressed as graphs).

Its claims, in the order I want them attacked:

```
C1  pii's engine is AHEAD of Infinity on exactly-once: `is_effect` per node kind plus a replay
    refusal, where Infinity's answer is per-tool dedup state in a datastore.
C2  pii is BEHIND Infinity on mid-turn resumability: a `coder`/`agent` node spawns an external agent
    and waits for the whole turn, so a daemon death loses the turn (G1).
C3  `tool` is synchronous, so a long command cannot be a node without a live run (G2).
C4  `spawn` starts a child run but has no report path back to the parent (G3).
C5  an external event (MQTT, cron, Discord) cannot reach an agent MID-TURN without breaking
    one-call-one-result ordering (G4).
C6  pii should NOT take Infinity's AWS stack, its actor-per-thread model, or its crate as a dependency.
C7  P1 (a coder turn persisted per tool call, resumable from the journal) is the highest-value port.
```

## Do this first: judge each system on its own

**Infinity** — from the primary source, not from my summary: `github.com/hydro-project/infinity`,
`infinity.hydro.run/docs/infinity-runtime/architecture`, `/docs/rap/what-is-rap`, and `/docs/rap/spec`.
Say what its actual guarantees are, what it deliberately leaves to the application, and where its
design would fail under pii's constraints.

**pii** — from the code in this repo: `crates/pii-server/src/engine/` (journal, dispatch, resume),
`crates/pii-server/src/node_kinds/nodes/coder*`, `.../tool.rs`, `crates/pii-graph/src/nodes.rs`
(spawn/wait/gate), `crates/pii-store/src/` (journal, replay). Say what pii actually guarantees today,
where the guarantee stops, and whether C1–C5 are true **as statements about this code**.

Cite file:line for every claim about pii. If you cannot check something, say so rather than infer.

## Then compare, on axes that decide adoption

```
1  DURABILITY      what survives a process death, and at what granularity (run · node · tool call)
2  SIDE EFFECTS    who owns exactly-once, and what the failure mode is when it is wrong
3  CONCURRENCY     what an idle unit costs; how many can exist; what ordering is guaranteed
4  EXTENSIBILITY   how a new capability is added: a node kind here, a tool there
5  OPERABILITY     one binary vs a distributed stack; what the operator must run
6  THE LONG WORK   a 3-day build, a 2-hour agent turn, a model call that hangs
```

For each axis: which system is better **for pii's stated purpose** (a single binary on a host, work as
graphs, only the commit left to a human), and what pii would have to change to close the gap — named as
a primitive, not as a wish.

## Judge my analysis

For each of C1–C7: **confirmed · refuted · refined**, with the evidence. Then:

```
A  what I MISSED — a property of Infinity that matters for pii and I did not mention
B  what I OVERCLAIMED — a sentence in 048 that is stronger than the code supports
C  what I got BACKWARDS — a comparison where the winner is the other system
D  the ranking you would give P1–P5 by (value to pii ÷ effort), and any P6 you would add
```

## Constraints

- Do not accept my gap list. Derive your own from the two systems and say which of mine survive.
- Do not propose adopting Infinity as a dependency; pii must run standalone. If you think that is
  wrong, argue it explicitly — it is a product decision, not a technical one.
- Distinguish what pii has TESTED from what it merely implements. Run nothing expensive; reading is
  enough, and say where you could not verify.
- Be concrete about cost: which of P1–P5 is a day, which is a week, which is a rewrite.

## Structure your answer as

```
1  INFINITY, AS IT ACTUALLY IS        guarantees · what it leaves to the app · where it would fail here
2  PII, AS IT ACTUALLY IS             guarantees today · where they stop · file:line evidence
3  THE SIX AXES                       per axis: winner, why, what pii must change
4  MY CLAIMS C1–C7                    confirmed / refuted / refined, with evidence
5  WHAT I MISSED · OVERCLAIMED · GOT BACKWARDS
6  THE RANKING                        P1–P5 by value÷effort, with a cost class each, plus any P6
7  THE ONE ROW TO TAKE NEXT           and the sentence pii could honestly say afterwards
```
