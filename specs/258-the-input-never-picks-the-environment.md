# Spec 258: the run input can never choose the agent's environment — the test that proves it can fail

## Overview
**Project**: <repo>
**Phase**: production — generic spec graph (spec 257 follow-up found by its independent verification, 2026-10-24)

Spec 257 lets `$.input.*` supply `agent`, `model`, `spec`, `workdir`, `brief` and four optional keys, and forbids resolving `env_pass`, `criteria`, `timeout_ms`, `container_image`,
`container_runtime`. Its test `env_pass_is_never_resolved` uses the ARRAY form `env_pass = ["$.input.names"]`. The resolver (`coder/input_cfg.rs`, `expr_of`) only resolves STRING values that
start with `$.`, so an array is skipped whatever the key list says. Measured by mutation: adding `"env_pass"` to the resolved keys leaves all 7 tests green. The dangerous form is the STRING one,
`env_pass = "$.input.names"`: if `env_pass` were ever resolvable, the input would hand the node an array of env names and widen what the agent sees. Nothing tests that.

## Requirements
- **R1.** New file `crates/pii-server/tests/coder_input_never_picks_the_environment.rs` (do NOT edit `coder_reads_the_run_input.rs`; copy the few helpers). Tests below, each RUNNING the node with a fake agent that records the environment names it sees.
- **R2.** `a_string_env_pass_is_not_resolved_from_the_input`: cfg `env_pass = "$.input.names"` (a string), input `names = ["OLLAMA_API_KEY"]`, the daemon env holds `OLLAMA_API_KEY`. The fake agent does NOT see `OLLAMA_API_KEY`.
- **R3.** `a_literal_env_pass_still_passes_its_names`: cfg `env_pass = ["OLLAMA_API_KEY"]` (literal array), same env. The fake agent DOES see it. (CONTROL for R2: the fake can see a name when it is allowed to.)
- **R4.** `criteria_and_image_and_runtime_strings_are_not_resolved`: cfg with `criteria = "$.input.c"`, `container_image = "$.input.i"`, `container_runtime = "$.input.r"` as strings and an input supplying them. The node behaves as if each were the literal text (no criterion runs, no container starts, no runtime is chosen from the input): assert the fake agent ran on the host path and the journal names no image.

## TDD Contract
| Test name | Expected |
|---|---|
| `a_string_env_pass_is_not_resolved_from_the_input` | the agent's recorded env does not contain `OLLAMA_API_KEY` |
| `a_literal_env_pass_still_passes_its_names` | the recorded env contains `OLLAMA_API_KEY` |
| `criteria_and_image_and_runtime_strings_are_not_resolved` | host path, no image in the journal, no criterion run |

**The requirement a plausible implementation satisfies only in appearance:** a test that passes because the form it uses is skipped for an unrelated reason. R2 uses the STRING form on purpose; R3 is the control that the environment check is live.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_input_never_picks_the_environment 2>&1 | grep -qE "^test result: ok\. [3-9] passed"`
- [ ] `cargo test -q -p pii-server --test coder_reads_the_run_input 2>&1 | grep -qE "^test result: ok\. 7 passed"`
- [ ] `git diff --quiet -- crates/pii-server/src crates/pii-server/tests/coder_reads_the_run_input.rs`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT edit any file under `crates/pii-server/src` or `coder_reads_the_run_input.rs`: this spec adds a test file only.
- Do NOT commit.
