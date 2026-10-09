# Spec: 89 — the gate compiles the llama provider (N4)

**Project**: /projects/pi-improved
**Phase**: 045 re-review, candidate 5 — a coverage hole spec 76 opened.

## Context
Spec 76 put `llama-cpp-2` behind a `llamacpp` feature (correct: the default build must not link
llama). But `quality-gate.sh` never enables that feature, and `real_granite`/`ds4_inprocess` carry
`required-features = ["llamacpp"]`. Consequence, measured by the reviewer: the in-process llama
provider and its 9 real-model tests are compiled by **no gate step** — the suite's ignored count fell
from 10 to 1 for that reason alone — so they can stop compiling without anyone noticing, including
the open A6 cancellation item in that provider.

## Requirements
R1. `quality-gate.sh` gains one step that compiles AND lints the feature:
    `cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings`
    (or `cargo check` + `clippy` if clippy over the feature needs extra target isolation — say which
    in the report and why). It must be a FAILING step like the others, positioned before the summary.
R2. The 9 real-model tests stay `#[ignore]`d (they need a 2.2 GB GGUF); this spec is about COMPILING
    and LINTING the gated code, not about running the model.
R3. Prove the step has teeth: in a `git archive` export, inject a deliberate type error into
    `crates/pii-model/src/llamacpp.rs`, run the gate, and show it exits non-zero naming that file.
    Remove the injection afterwards (the export is throwaway, not the tree).

## Exit Criteria
- [ ] `[ "$(grep -c 'features llamacpp' quality-gate.sh)" -ge 1 ]`
- [ ] `bash quality-gate.sh` exits 0 (the new step passes)
- [ ] the injected-type-error proof: gate exit non-zero, output names `llamacpp.rs` (record verbatim)
- [ ] `cargo tree -p pii-model | grep -c llama` is still 0 (the DEFAULT build must not link llama — spec 76's property preserved)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 and `cargo fmt --all --check` exits 0
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] the report says whether the gate got slower and by how much (a rough number is fine)

## Guardrails
- Do NOT make `llamacpp` a default feature anywhere (that would relink llama into every build).
- Do NOT un-ignore the real-model tests.
- Do NOT touch the provider's logic — this is a gate-coverage spec.
- Scratch exports under `/projects/_tmp/` only, and clean up after yourself (one export ≈ 8G).
- Do NOT commit; leave the tree for the orchestrator.