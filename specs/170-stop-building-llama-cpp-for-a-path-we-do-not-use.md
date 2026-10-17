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
