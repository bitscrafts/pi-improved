# Spec: Every gate step uses the warm cache

## Overview

Measured 2026-10-17: not one step of the fresh gate uses the shared warm cache. `clippy`, `clippy (llamacpp)`,
`clippy (hal-gpiod)` and `test (doc)` set `CARGO_TARGET_DIR=target/<name>-gate` — RELATIVE, and the gate runs its steps
inside the freshly exported directory — while `build` and `test (lib/bins/tests)` set nothing at all. So every gate run
compiles the whole workspace from scratch, four times over in four separate target dirs: rustc at 12 minutes for a single
crate, three concurrent linkers, load 88 on 32 cores, and the account window frozen for two hours because compiling spends
no tokens. FRESH_GATE_TARGET exists and is used by NOTHING on this path.

**Project**: <repo>

## Requirements

Point every build step at the shared cache while KEEPING its own subdirectory (the separate dirs exist to avoid stale-rlib
E0463 across feature sets):

    clippy              CARGO_TARGET_DIR="$FRESH_GATE_TARGET/clippy-gate"
    clippy (llamacpp)   CARGO_TARGET_DIR="$FRESH_GATE_TARGET/llamacpp-gate"
    clippy (hal-gpiod)  CARGO_TARGET_DIR="$FRESH_GATE_TARGET/hal-gate"
    test (doc)          CARGO_TARGET_DIR="$FRESH_GATE_TARGET/doctest-gate"
    build               CARGO_TARGET_DIR="$FRESH_GATE_TARGET/build-gate"
    test (lib/bins/tests) CARGO_TARGET_DIR="$FRESH_GATE_TARGET/test-gate"
    (and make FRESH_GATE_TARGET default to a persistent path when unset, so a gate run is never accidentally cold)

The build step must keep publishing `target/debug/pii` inside the export, because the pii-worker supervisor tests spawn it
by that fixed path.

## Exit Criteria

- [ ] `grep -c 'FRESH_GATE_TARGET' gate-steps.txt` returns at least 6
- [ ] two consecutive `bash quality-gate.sh` runs: the second must finish in less than a third of the first

## Guardrails

- Do NOT weaken or remove any gate step; this changes only WHERE build artefacts live.
- Do NOT touch gate-steps.txt while a gate is running (quality-gate.sh reads it per step).
- Do NOT commit; the loop's orchestrator reviews and commits.
