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

## 7. Can a skill embed scripts and templates? (user question, 2026-10-19)

**Definition agreed the same day: from now on, "skill" means `DocKind::Skill`.** No other meaning is intended in this repo.

Measured shape of a memory document (`pii-memory/src/doc.rs`): `id · layer · kind · text · concepts · importance ·
created_ms · last_used_ms · evidence: Option<(String, u64)> · archived`. There is **no attachment field, no binary, no
executable slot** — a document is TEXT plus concepts, importance and an evidence pointer.

So the answer has three parts.

### 7.1 Yes, as text — and that is retrievable
A skill's body can contain fenced blocks: instructions, a shell snippet, a template, a diff. Because bm25 and the vector
lane index the text, a search for "template for a Rust spec" finds the skill and returns the fenced block with it.
`concepts` is the cheap tag surface: `concepts = ["skill/template/rust-spec", "trigger/dispatch"]`.

### 7.2 What must NOT be done
Memory text must never be EXECUTED verbatim:
- a document has no digest pinning, so any later `put` silently rewrites what would run;
- `evidence` is an optional pointer, not an integrity check;
- an edit to memory would change behaviour with no commit, no gate and no review — the opposite of every rule this project
  spent a week learning (1,846 runs from an unaudited path; 4 posts from cancelled runs).

### 7.3 The shape that keeps both properties
| part | lives where | why |
|---|---|---|
| intent, instructions, selection | `DocKind::Skill` document | recall drives selection; it is data, and it decays |
| the script / template itself | a file in the repo (or a mounted volume) named by `evidence` | it is versioned by git, reviewable in a diff, and hashable |
| the pin | one line in the skill body: `script: <path> sha256=<hex>` | the coder node verifies the hash before running; a drifted file is a REFUSAL |
| variants | separate `Skill` docs linked by an entity relationship | retrieval can pull the template alone; `pii memory link` + `neighbours` already do this |

The executable copy is therefore **never memory**: memory holds the skill's meaning and its pin, the repo holds the bytes,
and the coder node (inside podman, per spec 209) is the only thing allowed to run them — with the digest in the journal.

### 7.4 The rule in one sentence
> A skill may EMBED text (instructions, templates, snippets) and may REFERENCE scripts; it may never be the place a script is
> executed from, and every referenced script carries a sha256 pin verified before it runs.

## 8. Hooks as nodes, skills as subgraphs (user question, 2026-10-19)

### 8.1 A hook SHOULD be a node — a woven one, not an event bus
pii already contains the mechanism: the `sop` node kind (spec 113) exists to "weave the bookkeeping nodes around each step".
That is a hook implemented as nodes, and it is strictly better than an event listener because:

| property | woven node | event bus |
|---|---|---|
| visible in the graph | yes — `graph check` shows it | no — implicit |
| can block | yes, a pre-node with an `err` port | only if the runtime enforces it |
| journalled | yes, its own start/finish | only if code remembers to emit |
| refuses loudly | yes, at load time | at fire time, if at all |

**Recommendation:** a hook is a DOCUMENT (config: event, mode, pin — spec 211) that the loader WEAVES into the graph as a
node at the named surface, at load time. `node.before` becomes a gate-shaped node on the incoming edge; `node.after` becomes
an observe node on the outgoing edge; `run.start`/`run.end` become the first and last nodes. The `gate.refuse` surface is the
`err` port of an existing gate.

Consequences, all good: hooks inherit the journal, the ports, the capability scope and `--network none` for free; a hook whose
pin drifted fails at LOAD (before anything runs) instead of vanishing at fire; and "which hooks run on this node" is answered
by reading the graph, not by reading a config file nobody audits.

The cost: weaving changes the graph the operator wrote, so `graph check` must PRINT the woven nodes and their origin
(`woven-from: hook/<id>`), or the graph stops being the truth.

### 8.2 A skill SHOULD be able to BE a subgraph
The podman skill's heal ladder — probe → inspect → act → verify → clean — is already a graph, not a paragraph. So a skill has
two possible bodies:

- **prose + snippets** (today): instructions a coder node reads and follows. Cheap, flexible, unverifiable in structure.
- **a subgraph** (new): a `.toml` in the repo, pinned by sha256 in the skill body exactly like a script, instantiated by a
  `subgraph`/`graph` node with the skill's inputs. Then "explain the skill" can print the STEPS, and "use the skill" runs a
  graph whose nodes are journalled and gated.

**Recommendation:** keep `DocKind::Skill` as the selection + explanation surface, and let its body pin EITHER a script OR a
subgraph file. Same rule as §7: memory holds the meaning and the hash, the repo holds the bytes, the coder/subgraph node runs
them.

### 8.3 The one thing neither may become
A second scheduler. A hook woven into a graph is still executed by pii's scheduler; a skill that is a subgraph is still one run
with one budget. The moment a skill or hook acquires its own timer and its own store, this project relives 2026-10-17: one cron
entry, four armed copies, 1,846 runs at one occurrence.

### 8.4 What this changes in the queued specs
- **211 (hooks)**: the event names stay as the DECLARATION surface, but the implementation shape is weaving into the graph at
  load time, with `graph check` printing every woven node and its `woven-from: hook/<id>` origin.
- **210 (skills)**: `explain` gains one line — `body: prose` or `body: subgraph <path> sha256=…` — so the reader knows whether
  the skill describes work or RUNS work.
