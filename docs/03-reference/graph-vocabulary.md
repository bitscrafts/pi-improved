# The graph vocabulary — measured, and the decision it needs

Status: **proposal, not a decision.** Nothing here is ratified. It exists so the operator can rule on
a table instead of on a description, and so the next person does not repeat the audit mistake below.

## The measurement

Parsed from the `[[node]]` tables of the 32 graph files under `graphs/` (2026-11-04, `tomllib`, not a
grep):

```
kind          graphs  nodes
transform         26     43
refuse            11     16
timer              8      8
call               7      8
coder              7      8
route              6     10
agent              5      6
gate               5      6
post               5      5
judge              4      8
tool               4      8
memory             3      4
heal_detect        2      2
recall             2      4
worker             2      2
classify           1      1     ← exactly one user
debug_crash        1      1     ←
enqueue            1      1     ←
heal_record        1      1     ←
plan               1      1     ←
record             1      1     ←
select             1      1     ←
wait               1      1     ←
```

23 kinds. Eight are used by exactly one graph.

**The audit that preceded this document was wrong**, and the error is worth recording: it grepped
`kind =` in the TOML, which matches *edge* kinds as well as node kinds. That produced `data` — a kind
that does not exist, from 120 edge declarations — and it missed `coder` and `agent`, the two kinds
that actually call a model. A grep cannot distinguish a node from an edge; a TOML parse can.

## What the kinds are for, grouped by function

```
CONTROL (pure, no world)      transform · route · refuse
COMPOSITION                   call
EFFECTS (touch the world)     timer · tool · post · judge · coder · agent · gate · memory · recall
OPERATIONS (one flow each)    select · record · plan · enqueue · classify · worker ·
                              heal_detect · heal_record · debug_crash · wait
```

## The decision each kind needs

Verdicts below are **proposals**, with the reason. "Derive" means the flow is expressible with the
base set and the kind is deleted; "merge" means two kinds are one; "admit" means it is a primitive and
stays; "widen" means a neighbouring primitive grows instead of a new kind appearing.

| Kind | Proposal | Why |
|---|---|---|
| `transform` | **widen, then admit** | It computes almost nothing today: expressions are member access only, no functions, no arrays. The engine needs a real value-computation primitive. This is the largest gap. |
| `route` | **admit** | Branching is primitive. It is also where a command's stdout meets a rule, so it owns the trim that was just fixed. |
| `refuse` | **admit** | A terminal negative is a verdict, not a failure. |
| `call` | **admit** | The growth operator. Pinned, static target. |
| `timer` | **admit** | Time is not derivable from anything else. |
| `tool` | **admit** | The one unrestricted effect; everything else must be more specific than it. |
| `post` | **admit** | Telling a human is a distinct effect from running a command. |
| `judge` | **admit** | Executing criteria is the system's central act. |
| `gate` | **admit** | A durable await that survives restart cannot be built from `tool`. |
| `coder` / `agent` | **merge to one** | Both call a model; `plan` calls the provider directly too, so the model call is one primitive wearing three names. |
| `memory` / `recall` | **merge, then admit** | One reads, one writes, same resource and same capability. |
| `worker` | **derive** | Both of its uses mean `coder`. |
| `classify` | **widen `route`** | Pure, no resource: it is a branch with several arms. |
| `plan` | **derive** | It is a model call plus string building: `coder` + the computation primitive. |
| `enqueue` / `record` | **fold into the queue family** | Both are one store write on the row table; the write belongs with `select`. |
| `select` | **admit as the queue primitive** | One transaction, four ports, a lease by run id. |
| `heal_detect` / `heal_record` | **merge** | One flow: read the gate's verdict, act, record the act. |
| `debug_crash` / `wait` | **examine, then derive or admit** | One user each, and neither has been exercised in production. |

Taken together: **23 kinds → about 11**, plus one genuinely new primitive (value computation).

## What is missing, in the order it blocks work

1. **Value computation.** Without it, "compute a stamp, compare it, route on the difference" is not
   expressible, and every such graph reaches for a shell string or a bespoke verb.
2. **Durable await** — exists as `gate`, but under-described; it should be a first-class, documented
   primitive rather than a heal-only device.
3. **A typed effect over a daemon-owned resource.** `recall` and `memory` are the working examples;
   the capability that guards them (`proc` standing in for `memory_read`) is a placeholder.
4. **An agent/model call as one primitive** — see the `coder`/`agent`/`plan` merge.
5. **Error/handler scope.** A refusal no host wires fails the run; the host must be able to catch it.

## Is a refactor warranted?

Yes, and its shape is *subtractive*: merge three pairs, derive eight single-user kinds, widen `route`
and `transform`, and add one primitive. That is smaller than the current module and strictly more
capable. What it must **not** become is a rewrite of the engine: the splice, the exit rule, pins and
the capability ladder all work and are covered by tests (`crates/pii-graph/tests`).

## What the operator must rule on

1. The base set itself — the twelve kinds in the rows above marked admit / merge / widen.
2. Whether eight single-user kinds may be deleted, or must first be shown to be unsupported flows.
3. Whether `tool` stays unrestricted (it makes the set "sufficient" trivially, which is how the
   earlier analysis fooled itself) or gains a declared, reviewable command surface.
4. Whether the value-computation primitive is an expression language in the graph file, or a
   `compute` kind, or a step toward a small typed language — this is the decision with the longest
   consequences.
