# Spec: hooks are documents you can audit

## Overview
User directive 2026-10-19: "also, lets do the same with the hooks" — i.e. hooks get the same treatment as skills: a hook is a
DOCUMENT in pii's memory, with a pinned script, and pii can list, show, explain and edit it. Today pii has no hook surface at
all (measured at `b2e2206`: `sched/dispatch.rs` has no before/after hook, only `on_resume` and effect idempotency), so the
first version must be born auditable rather than bolted on later. Hermes' own history is the warning: a hook that can be
disabled silently, or that runs on an event nobody enforces, is a hook of no use.

**Project**: /projects/pi-improved

## Verified premises
- `DocKind = Atom | Scene | Persona | Skill` (`pii-memory/src/doc.rs:21`) — there is no Hook variant yet.
- `Doc` carries `text · concepts · layer · evidence(source, ms)` and no executable slot; skills therefore pin a script by
  sha256 in the body (`_docs/03-reference/skills-and-hooks-in-pii.md` §7).
- Node ports already provide the failure edges (`err`, `escalate`, `inconclusive`, `flake`); the journal records every node
  start/finish, which is what a hook has to hang off.

## Requirements
### Functional
1. `DocKind` gains **`Hook`**. A hook document's body declares, on its own lines:
   `event: <surface>` · `when: <predicate>` · `mode: block|observe` · `script: <path> sha256=<hex>`.
2. **Five event surfaces only**, all named constants (no free-form strings): `node.before`, `node.after`, `run.start`,
   `run.end`, `gate.refuse`. A hook naming any other event is REFUSED at load (`hook.event.unknown`, naming the event).
3. **Only PRE surfaces may block** (`node.before`, `run.start`); `mode: block` on a POST surface is REFUSED at load
   (`hook.mode.invalid`). This is Hermes' hard-won lesson encoded from the start: an event that cannot block must not pretend
   to. POST hooks and `gate.refuse` are `observe` only.
4. `pii hook list [--all]` — id · event · mode · script · pin status (`ok|DRIFT|MISSING FILE`) · last fired.
   `pii hook show <id|slug>` — body verbatim. `pii hook explain <id|slug>` — rendered from the body: EVENT, WHEN, MODE,
   SCRIPT+PIN (verified), and the last three journal entries it produced.
5. `pii hook edit <id> --file <path>` — same contract as skills: keeps the id, records evidence, REFUSES an edit that drops the
   pin, changes the event to an unknown one, or sets `block` on a POST surface.
6. Every hook execution is journalled as its own event `hook.fired{hook=<id>, event=…, outcome=…}`; a hook with no journal
   entry did not run, and `explain` must show that absence rather than claim success.

### Non-Functional
- `DRIFT` or `MISSING FILE` pins make a hook INERT: it is loaded, listed, explained and refused at fire time
  (`hook.pin.drift`), never silently skipped.
- A hook may not spawn a process outside podman (spec 209) and may not install anything.
- Loading hooks never mutates the graph: hooks attach to events, they do not become edges.

## TDD Contract
| Test | Input | Expected |
|---|---|---|
| hook_kind_exists | a Hook document | listed by `pii hook list`, absent from `pii skill list` |
| unknown_event_refused | `event: node.exploded` | `hook.event.unknown`, hook inert |
| block_on_post_refused | `mode: block` + `event: node.after` | `hook.mode.invalid` |
| pre_hook_can_block | `node.before`, mode block, script exits 1 | node does not run; `hook.fired{outcome=blocked}` |
| observe_hook_cannot_block | `node.after`, script exits 1 | node result unchanged; outcome recorded as `observed` |
| drift_makes_it_inert | pin hash wrong | `hook.pin.drift`, no execution, still listed |
| explain_shows_absence | hook never fired | explain prints "no journal entries" rather than a success line |

## Exit Criteria
- [ ] `cargo test -p pii-server --test hook_documents`
- [ ] `cargo test -p pii-server --test hook_pre_can_block_post_cannot`
- [ ] `pii hook list` on the live store shows at least the podman guard hook with `ok`
- [ ] `cargo test -p pii-memory --test dockind_hook` (the variant round-trips through serde as `hook`)

## Guardrails
- Do NOT allow a hook to be a shell string: it names a script, and the script is pinned and hashed.
- Do NOT let a POST hook block, and do NOT let an unknown event load.
- Do NOT run a hook whose pin drifted; inert and visible beats skipped and silent.
- Do NOT invent a second hook store: hooks are memory documents, like skills.

## Depends on
- 210 (the same list/show/explain/edit surface, plus the pin verification helper) — the two share one audit path.

## Implementation shape (added 2026-10-19 after the design question)
Hooks are **woven into the graph at load time as nodes**, not dispatched through an event bus — the `sop` node kind (spec 113)
already proves the weaving pattern exists in this codebase. Concretely:
- `node.before` → a gate-shaped node on the incoming edge (its `err` port is the block)
- `node.after` → an observe node on the outgoing edge (cannot block, by construction rather than by validation)
- `run.start` / `run.end` → the first and last nodes of the run
- `gate.refuse` → handled on the gate's own `err` port

`graph check` MUST print every woven node with its origin `woven-from: hook/<id>`, or the graph stops being the truth. A hook
whose pin drifted fails at LOAD, not at fire.
