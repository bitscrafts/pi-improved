# Spec: 113 — the SOP engine: declarative procedures compiled into the graph

**Project**: /projects/pi-improved
**Phase**: Stage A of the vision plan (`057`), third row. The vision's "SOP engine" (pillar 2) — and the
surface that makes pii useful for repeated work rather than one-off prompts.

## Design decision (fixed by the plan; do not re-litigate)
An SOP **compiles into the existing graph**. The engine already has: a DAG with bounded cycles, journal-based
resume, `Suspend` as a node return, capability-gated `Ctx`, and an approval store whose ids are run-scoped
(spec 106) and whose re-request is idempotent (spec 88). A second execution engine would duplicate all of it
and diverge. So: a declarative SOP file → a deterministic graph → `pii graph run`, plus `pii sop` verbs for the
human.

## What exists (measured 2026-10-10; do not re-derive)
- Graph TOML with `[[node]]`/`[[edge]]`, `[limits]`, `entry`, `caps` (see `graphs/` and `fixtures/`).
- Node kinds include `agent`, `gate` (approval, suspends), `transform`, `emit`, `wait`, `memory`, `heal`,
  `spawn`, `worker`, `crash` (`crates/pii-server/src/node_kinds/nodes/` + the graph-core kinds).
- `pii graph {lint,show,run,resume,trace}` exists; the journal is the run's record.

## Requirements
R1. **An SOP file format** (TOML, alongside graphs): an id, a title, `caps`, and ordered `[[step]]` entries
    with at least: `id`, `kind` (`act` = an agent turn with a prompt/graph ref; `check` = a verifier that must
    pass; `approve` = a human gate; `notify` = a message out), `requires` (preconditions), and per-step
    inputs. Keep it small and declarative: the SOP describes WHAT, the graph machinery does HOW.
R2. **A deterministic compiler** `sop → graph TOML` (`pii sop compile <file>` prints the graph), with
    `pii sop {lint,show,run,resume}` beside it. Determinism is a requirement: the same SOP compiles to the
    same graph bytes (a test asserts byte-equality across two compiles), because the graph is what the journal
    and the reviewers read.
R3. **Preconditions refuse loudly.** A `check` step or a `requires` clause that fails must end the run with a
    named failure (an event + a terminal status naming the step), never skip silently and never continue.
R4. **Approvals reuse the existing machinery**: an `approve` step becomes a `gate` node; the approval id stays
    run-scoped (spec 106) and idempotent across a lag (spec 88); `pii approve ls|allow|deny` works unchanged,
    and the SOP step id appears in the approval's `node`/prompt so a human knows WHAT they are approving.
R5. **Observability**: named events for `sop.step.started`, `sop.step.finished`, `sop.step.failed`,
    `sop.step.awaiting_approval` (register every name in `names.rs` + `ALL` — the completeness rule enforces
    it), carrying the SOP id and the step id in the fields, so a run's trajectory (spec 103) is readable as a
    procedure, not as anonymous nodes.
R6. **A worked example, checked in**: one real SOP (e.g. "cut a release": act → check → approve → act →
    notify) in `examples/`, and the proof that it lints, compiles deterministically, runs against the
    scripted model, suspends at the approval, and resumes to completion after the approval. Paste the
    journal/trace for the suspend and for the resume.
R7. **A guide**: `_docs/0X-guides/writing-an-sop.md` — the format, the compilation rule, the precondition
    semantics, the approval flow, and the example. Update `001`'s status line for the SOP engine (it currently
    implies it does not exist).
R8. **A control for the compiler** (this repo's standing rule): a mutation that drops or reorders a step must
    turn a test RED (paste it) — a compiler nothing pins is a compiler that drifts.

## Exit Criteria
- [ ] the SOP format + compiler exist; the same SOP compiles to byte-identical graphs twice (paste the hashes)
- [ ] `pii sop lint` refuses a malformed SOP (a missing step id, a duplicate id, a cycle) with a message per
      case (paste)
- [ ] the example SOP runs end to end with the scripted model: suspend at the approval, then resume after
      `pii approve allow <id>` (paste both traces)
- [ ] a failing precondition ends the run with the step named (paste), and the negative control (a passing
      precondition continues) is pasted too
- [ ] the four new event names are emitted in that run and present in `ALL` (paste the lines)
- [ ] the compiler mutation turns a test RED (paste)
- [ ] `_docs/0X-guides/writing-an-sop.md` exists and `001`'s SOP line is updated
- [ ] `bash quality-gate.sh` exits 0 (`PASS: lint-observability`, `PASS: coverage-floor` or the loud skip);
      `fresh-gate --worktree` prints `fresh-gate: PASS`; clippy `-D warnings` and `fmt --check` exit 0
- [ ] the report lists files changed, new name consts, and the exact `pii sop` usage lines

## Guardrails
- **No second execution engine.** If you find yourself writing a scheduler, stop: compile to the graph.
- Determinism is not optional (a compiler that emits key-order noise makes every diff unreadable).
- An SOP must never be able to widen its own capabilities: `caps` come from the SOP file and are attenuated
  by `Ctx` as usual — say in the guide that this is where authority comes from.
- Approval semantics are the existing ones; do not add a second approval store or a second id scheme.
- Scratch under `/projects/_tmp/`; keep it small, delete your own, no daemon left running.
- Do NOT commit; leave the tree for the orchestrator.
