# 075 — Design brief for review: skills, hooks, fragments, installed-graph verbs, and graph CATEGORIES

Written 2026-10-19 for a Claude+Opus analysis pass. Everything below is either a user directive (quoted), a measurement, or a
spec already committed. Nothing here is implemented except where it says so.

## 0. How to read this

Three of these are **specified and queued** (210, 211, 212), one is **proposed because it came from the user's ideas**
(categories), and one is the **audit surface** for what the daemon actually loads (213). The question for the review is not
"is this nice" but **"what breaks if we build it in this order, and what is missing from the contract"**.

## 1. The four surfaces

### 1.1 Skills — `DocKind::Skill` (user, 2026-10-19: "from now on when i mention skill, thats what it will mean")
- a skill is a MEMORY DOCUMENT: id, layer, kind, text, concepts, importance, evidence — no executable slot
- it may EMBED text (instructions, templates, snippets) and may REFERENCE scripts
- the executable bytes live in a repo file; the body carries `script: <path> sha256=<hex>`; drift is a REFUSAL
- a skill MAY also pin a SUBGRAPH instead of prose (`body: subgraph <path> sha256=…`) — the podman heal ladder is already a
  graph written as a paragraph
- **spec 210** (queued A78): `pii skill list | show | explain | edit`, with pin verification in list and explain

### 1.2 Hooks — documents woven as nodes (user: "also, lets do the same with the hooks")
- a hook is a document (new `DocKind::Hook`) declaring `event · when · mode · script+sha256`
- **five events only**: `node.before · node.after · run.start · run.end · gate.refuse`
- **block only where blocking is real**: `mode: block` allowed on PRE surfaces only; refused on POST
  (Hermes' lesson: there, only `pre_tool_call` can block and fail_closed was silently ignored everywhere else)
- **implementation shape: WEAVING at load time**, not an event bus — `sop` (spec 113) already proves the weaving primitive.
  `node.before` → gate-shaped node on the incoming edge; `node.after` → observe node on the outgoing edge;
  `run.start/end` → first/last nodes. `graph check` must print `woven-from: hook/<id>`
- every fire is its own journal event: `hook.fired{hook, event, outcome}`; a hook with no entry did not run
- **spec 211** (queued A79)

### 1.3 Fragments — a fragment attachable to any graph (user: "so we could have a generic skill or hook graph that can be attached on any other graph!" — asked as a QUESTION)
- `[[attach]] fragment = "hooks/podman-guard" at = "run.start" on = ["*"] pin = "<sha256>" [order = N]`
- fragment file under `graphs/fragments/` with `[fragment] name · inputs · outputs · may_block`
- resolved at LOAD: namespaced ids `<attach>.<node>`, stamped `woven-from`, printed by `graph check`
- rules: pin verified · `may_block=false` may not attach at a PRE surface · **no timer, no store, no budget in a fragment** ·
  **no nesting (depth 1)** · deterministic order · failure routes to the fragment node's own `err` then fails the run naming
  fragment+node
- **spec 212** (A80) — STILL A PROPOSAL because the user's sentence was a question; not to be built without a decision

### 1.4 Installed graphs — the audit surface (user: "the pii also must be able to list/edit and explain its installed graphs")
- today `pii graph` = `lint | show | run | resume | trace`; **no list, no edit**
- installed on lnx: `discord-digest · discord-triage · intervention-report` (3); in the repo: 19 — the two sets have already
  diverged, and nothing reports it
- **spec 213** (A81): `list` (sha, nodes, trigger, armed runs, drift `same|DRIFT|repo-only|installed-only`) · `show` · `explain`
  (rendered from the file: TRIGGER, node chain with kinds+ports, EFFECTS, CAPABILITIES, woven fragments, drift + hash) ·
  `edit` (LINT first · refuse on armed run unless --force · move previous aside `.prev-<epoch>` · atomic write · re-lint ·
  print `before-sha → after-sha`)

## 2. The new idea: GRAPH CATEGORIES (user, 2026-10-19)

> "a graph category, immutable system graphs, user graphs, project graphs!"

Proposed model, to be attacked:

| category | where it lives | who may write | editable | can be overridden | examples |
|---|---|---|---|---|---|
| **system** | shipped inside the binary / install tree, hashed at build | nobody at runtime | NO (immutable; a change is a release) | NO — a user graph cannot shadow a system graph of the same name | `queue`, `code-index`, `heal`, the recovery triage graph |
| **user** | `~/.local/state/pii/graphs` (per operator) | the operator via `pii graph edit` | yes, audited | yes — overrides a project graph of the same name | `discord-digest` schedule tweaks, personal watchdogs |
| **project** | `<repo>/.pii/graphs/` or `graphs/` in the project | the project's own commits | yes, via git | yes — overrides system? NO; overrides nothing but is the default for that project | `queue.toml` for THIS project, `code-index` for THIS repo |

Open tension worth the reviewer's attention:
- **precedence**: today a name resolves to one file. With three tiers, name shadowing becomes possible and *which one ran* must
  be a reading, not an assumption. Proposal: system > project > user for RESOLUTION, but a run names the exact file + hash, and
  `pii graph list` shows all three tiers with an explicit `shadowed` marker.
- **immutability**: "immutable" must mean *verified*, not *unwritable* — otherwise an operator edits it anyway and nothing
  notices. Proposal: system graphs carry a build-time hash; a mismatch is `graph.system.modified` and refuses to arm.
- **fragments across tiers**: may a user graph attach a system fragment? (Recommendation: yes — fragments are the reuse
  mechanism; a user graph attaching a *user* fragment that the system does not know about is the case that needs a rule.)
- **budgets**: a user graph that attaches a system fragment must inherit the run's budget, never bring one (212's rule).

## 3. Questions for the review

1. Is the three-tier model right, or is "system" really just "the graphs the installer ships" and the distinction is
   unnecessary bureaucracy?
2. What is the correct RESOLUTION rule when a name exists in two tiers, and how should `list`/`explain` prove which file a run
   used?
3. Is weaving (hooks and fragments) the right implementation, or does it make the graph unreadable at scale? What is the limit —
   5 woven nodes, 50, 500 — and what breaks first?
4. A skill may be prose or a subgraph. Is that one concept or two pretending to be one? If two, what are the names?
5. What must NEVER be allowed in a user-tier graph (capabilities? agent+model? net grants? memory writes?), given that the
   podman skill is the repair tool and these graphs will run with real capabilities?
6. What is missing from the four specs that a cheap worker would implement wrongly if it read only the spec?
7. What must not be claimed until a coder node has run in production on lnx?

## 4. Hard constraints any proposal must respect

- pii installs nothing; podman is the tool (209, done: `d5315e1`, three tests green)
- a fragment/hook/skill may never own a timer, a store or a budget — one run, one budget
- every capability a graph needs is declared and visible in `explain`
- nothing is deleted: previous versions move aside and are named
- a worker never commits; the orchestrator commits after the gate
- memory holds meaning and hashes; the repo holds the bytes; the journal holds the evidence

## 5. Where this stands in the queue at time of writing

`A77 209 podman` DONE (`d5315e1`) · `A78 210 skills` · `A79 211 hooks` · `A80 212 fragments` (proposal) · `A81 213 installed-graph verbs`
