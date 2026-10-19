# Skills and hooks in pii — what exists, what does not, and the shape they should take

Written 2026-10-19, after the user asked: *"actually does pii has access to skills? hooks pre/pos?"*
Every line below is a measurement against the tree at `b2e2206`, not a plan.

## 1. What pii does NOT have

**No skills system.** `grep -rl skill crates/` returns exactly three sites:
- `pii-memory/src/doc.rs:28` — `/// A learned skill.` (a `DocKind`, i.e. a memory DOCUMENT KIND)
- `pii-memory/src/decay.rs:8` — `/// λ for skills.` (a decay constant for that kind)
- `pii-model/tests/real_granite.rs` — a test fixture string

So a "skill" is a document that decays slowly. Nothing loads it, selects it, or executes it. There is no registry, no
frontmatter, no scripts directory, no selector.

**No pre/post hooks.** `sched/dispatch.rs` has no before/after hook surface. What exists instead:
- `on_resume` — a re-attach check before falling back to a fresh `run()`
- effect idempotency — journalled before the effect and confirmed after
- node **ports** — `ok`, `err`, `escalate`, `inconclusive`, `flake`, `detected`, `allow`

## 2. What pii has that those mechanisms would be built from

| mechanism | where | what it gives a "hook" |
|---|---|---|
| ports on every node | 31 `ok`, 4 `err`, 2 each of `escalate`/`inconclusive`/`flake`/`detected` | "what happens when it fails" is already an edge — the half of a post-hook that matters |
| `sop` node kind (spec 113) | `node_kinds/nodes/sop.rs` | a PRE/POST wrapper: `begin` checks `requires` + emits started, `end` emits finished, `fail` fails the run naming SOP and step — but only around SOP steps |
| journal | every node start/finish is an event | an external observer can act before/after a node without pii knowing |
| `gate` node | `node_kinds/nodes/gate.rs` | a checkpoint that can refuse and route |
| `heal` node | `node_kinds/nodes/heal.rs` | a failure ladder, escalating on evidence |

Node kinds at `b2e2206`: `coder · queue · gate · heal · memory · post · sop · spawn · classify · entity · crash · hal ·
mqtt · wasm`.

## 3. The honest one-sentence summary

> pii can gate, refuse, escalate and record around a node; it cannot run a declared SKILL (a document with instructions
> and commands), and it cannot hook arbitrary pre/post behaviour onto an arbitrary node.

## 4. The shape each should take (recommendations, not decisions taken)

### Skills → memory documents + `sop` steps
- a skill is a `DocKind::Skill` document whose body carries instructions and commands, versioned by content
- a `sop` step reads it (`memory` node, `op=search`) and hands the text to a `coder` node
- exactly the pattern already used for `model`: the value is DATA in the graph, never code in Rust
- selection is memory recall (bm25 + vector + graph lanes), which already works — no new registry
- do NOT add a second filesystem of skills: the memory store is the registry, and a duplicate is a second truth

### Hooks → ports and a `hook` node kind
- **graph edges + ports** (exists today) is hook shape (a): visible, journalled, bounded
- **a `hook` node kind** is the `sop` node generalised from SOP steps to any node — it wraps, checks `requires`, emits its
  own events, and routes failure to `err`
- **never** arbitrary shell pre/post hooks: a hook that runs commands outside the graph is unauditable, and this project has
  already produced 1,846 runs from one unaudited path (cron arming) and 4 posts from runs that were cancelled

## 5. Guardrails if these are built

- no skill may carry a credential; secrets come from `env_pass` names (spec 209)
- a hook may not spawn a process outside `podman` (209) and may not install anything
- every hook's own work must appear in the journal with its own node id, or it is not a hook — it is a hidden side effect
- skills are memory documents, so they must be recallable by the same search that recalls knowledge — verify with one
  `pii memory search` before claiming the mechanism works

## 6. Where this sits in the queue

Nothing in this document is queued. The related open work is A77 (spec 209 — pii installs nothing, podman is the tool),
which is the prerequisite for a coder node that could consume a skill, and 207/208 which are committed at `b2e2206`.
