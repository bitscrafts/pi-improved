# Spec: stop building llama.cpp for a path the product does not use

## Overview

Measured 2026-10-17: `llama-cpp-2` is an OPTIONAL feature of pii-model (`llamacpp = ["dep:llama-cpp-2"]`), it is OFF in
every build step except one, and the product reaches models over HTTP — `llamacpp-server:<model>` is an HTTP endpoint
(pii-model/src/lib.rs:267). pii never runs inference in-process. Yet the gate carries a step `clippy (llamacpp)` that
compiles llama.cpp from source with CMake, which is historically the longest step of the gate and the reason clang, cmake,
make and ninja had to be installed in this container at all. One gate run costs 10-25 minutes of C++ compilation for a
code path the daemon never executes.

**Project**: /projects/pi-improved

## Requirements

1. Remove `clippy (llamacpp)` from the ROUTINE gate steps. The contract it protects (that the in-process binding still
   compiles) is real but does not belong on every run: keep it as an explicit, manually invoked check, documented in
   `_docs/05-operations/deploy-runbook.md`.
2. Audit whether `llama-cpp-2` should remain a dependency at all. If nothing in the runtime, the daemon, the CLI or the
   graphs uses in-process inference, make it a dev-only, opt-in feature (or remove it), and record the decision with the
   evidence that the HTTP path is the only one exercised.
3. Whatever the outcome, `clang`, `cmake`, `make` and `ninja` must stop being prerequisites of a routine gate run, or the
   document must state why they are still needed.

## Exit Criteria

- [ ] `grep -c 'llamacpp' gate-steps.txt` returns 0 or 1, and the remaining reference is a comment explaining the manual check
- [ ] `bash quality-gate.sh` completes without compiling any C++ (evidence: no `clang++`/`cmake` in the run's process list)
- [ ] a two-line note in `_docs/05-operations/deploy-runbook.md` saying how to run the in-process binding check on demand

## Guardrails

- Do NOT delete the `llamacpp` feature or its tests; make it opt-in, and keep the code path compiling when it is asked for.
- Do NOT touch gate-steps.txt while a gate is running (quality-gate.sh reads it per step).
- Do NOT commit; the loop's orchestrator reviews and commits.

## Closure

Closed 2026-10-17T18:22Z. Work committed `5ffb8a5` (gate-steps.txt, quality-gate.sh, deploy-runbook.md); the plan row
that could not commit it is `e42584f`.

- **Criteria run by the orchestrator on the tree the worker left** (session `e5de5b89-8dde-4c1d-b27b-d2151766553b`),
  not taken from the worker's report: all three green — `llamacpp` appears once and only as the pointer to the manual
  check, no `clippy (llamacpp)` step remains, and the runbook carries the on-demand command.
- **Gate, on that same tree:** `fresh-gate` PASS four times between 18:05Z and 18:15Z (cycles `…180517Z`, `…180810Z`,
  `…181025Z`, `…181320Z`); the run's step list is fmt, clippy, build, with no llamacpp step and no clang++/cmake work.
- **Reddened with the orchestrator's own mutation:** appending
  `clippy (llamacpp)|cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings` to gate-steps.txt took
  criterion 2 to rc=1; the line was removed and the file confirmed byte-identical (`sha256sum -c` OK, `590a1637…`).
- **Why four green gates produced no commit:** the plan row named `crates/pii-graph/src/journal.rs`,
  `…/sched/dispatch.rs` and `…/tests/deadline_flush.rs` — a neighbour's files, with one criterion where the spec has
  three. `git add` therefore staged nothing and the cycle's only remaining exit was to close the item as "satisfied by"
  an unrelated commit. The row now carries the three paths the work touches and the three criteria verbatim.
  A36-A40 all shared that copy-pasted paths column; `3d9bb05` corrects it.
