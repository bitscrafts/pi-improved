# Spec: methodologies are graphs, and the night fills as well as drains

## Overview

Two corrections to the earlier draft of this row, both from the operator:

> sdd is retired (what happens if using other methodology)? night should also fill the queue!?
> those must be well thought

Both are right, and the draft was under-thought in the same way twice: it **deleted a capability**
instead of composing it, and it made the night **half a loop**.

## The correction, in layers

```
EFFECTS        the nine (spec 383): data · transform · route · call · refuse · timer · tool ·
               spawn · post · judge
METHODOLOGIES  how a piece of work is proved. Each is its own graph, pinned when called:
                 sdd          contract → call tdd → verify      (RED before GREEN: the strict one)
                 method-judge implement → judge the criteria    (the judge path, extracted from spec)
                 (a third is a matter of writing one: implement → external review → accept)
DRIVER         spec: read the spec → route on the methodology → call ONE of them (pinned) →
               call docs → verify → done | unverified | refused
CLOCK          night: tick → fill (call self-improve) → drain (call queue) → review → report
```

`sdd` is **kept**. It was never a duplicate of `spec`; it is the spec-driven *methodology*, and `spec`
is the driver that selects one. What was wrong before is that the driver had a methodology baked into
it (`coder → judge` inline), which is why a second path appeared to exist. The fix is to make the
judge path a graph too and let the driver route.

The night's loop is the one `_docs/05-operations/the night loop` already describes: **fill the queue,
drain it, review, refill.**

## Open questions, with the answers this row takes

- **Q1 — who chooses the methodology?** The spec may declare `method:` in its premises (a spec that
  names a test file is asking for `sdd`); a caller may override it through the run input; otherwise
  the driver derives it: `sdd` when the spec names an existing test path, `method-judge` otherwise.
  The chosen methodology is journalled, so the report can say which was used.
- **Q2 — when does the night fill?** Fill runs when the queue is empty, and only then. Filling every
  tick would re-plan work that is already queued; never filling is the defect this row fixes.
- **Q3 — can the methodology be a dynamic call target?** No: every `call` target in the repository is
  static and every call carries a `sha256` pin (measured: 0 dynamic targets). The driver therefore
  routes between **two pinned calls**, so each methodology keeps its pin and a review of the pin is
  still a review of a specific file.

**Project**: <repo>
**Phase**: composability


## Amended by derivation 047 (measured, not preferred)

- **`tdd` merges INTO `sdd`**, not into `spec`: `sdd` keeps the contract and the RED/GREEN proof and
  loses only the `docs` step, which the driver owns. `spec` becomes the driver: choose a method, call
  it, document, re-judge. `method-judge` is the second method and is new.
- **`night` fills as well as drains**: `tick → call self-improve (fill when empty) → call queue
  (drain) → review → report`. A night that only drains is a clock; a night that never fills stops the
  moment the queue is empty, which is the state this host has been in for hours at a time.
- **Refusals must be wired** at every call: `sdd` can return `refused` and `unverified`, and `night`
  must say what it does with each — row 364's defect one layer up, still open in `night.toml`.

## Requirements

- **R1** `graphs/spec.toml` is a driver: it holds no coder and no judge of its own. It routes on the
  methodology and calls one of the pinned methodology graphs, then `docs`, then verifies.
- **R2** `graphs/sdd.toml` remains, unchanged in role: the methodology that proves a test was red
  before the code existed.
- **R3** `graphs/method-judge.toml` exists: the judge path extracted from today's `spec`, so the two
  methodologies are peers and neither is inline in the driver.
- **R4** `graphs/night.toml` calls `self-improve` (fill) and `queue` (drain), in that order, and
  reports what it filled and what it drained.
- **R5** The chosen methodology appears in the run's journal, so a report can state which one was used.
- **R6** A spec that names no test and declares no method runs `method-judge`; a spec that names an
  existing test path runs `sdd`. Both are tested.

## Exit Criteria

- [ ] `test -f graphs/method-judge.toml`
- [ ] `test -f graphs/sdd.toml`
- [ ] `grep -q 'graph = "self-improve"' graphs/night.toml`
- [ ] `grep -q 'graph = "queue"' graphs/night.toml`
- [ ] `grep -qE 'method' graphs/spec.toml`
- [ ] `cargo test -q -p pii-server --test night_fills_and_drains 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test spec_selects_a_methodology 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`

## Guardrails

- Do NOT delete `sdd`: a retired methodology is a capability the operator loses without notice.
- Do NOT route by a dynamic `call` target: pins are static, and an unpinned methodology is a run
  nobody reviewed.
- Do NOT let the driver keep a private copy of a coder or a judge — that is how the duplicate appeared.
- Do NOT fill the queue on every tick; fill when it is empty, so a night does not re-plan its own work.
- Do NOT let a fill failure stop a drain: a night that cannot plan can still finish what it has.

## Files

- `graphs/spec.toml`
- `graphs/sdd.toml`
- `graphs/method-judge.toml`
- `graphs/night.toml`
- `graphs/self-improve.toml`
- `graphs/DOCS/method-judge.md`
- `graphs/README.md`
- `crates/pii-server/tests/night_fills_and_drains.rs`
- `crates/pii-server/tests/spec_selects_a_methodology.rs`
