# Spec 246: the agent's prompt names the spec it implements

## Overview
**Project**: /projects/pi-improved
**Phase**: post-vision hardening — agent management (defect 6 in `_docs/05-operations/agent-management-findings-2026-10-23.md`)

A `coder` node has a `spec` field (the path of the spec the agent implements) and a free-text `brief`. Today `spec` is used to
name pi's session file and to be non-empty, and nothing else: it never reaches the agent's prompt. Measured on the first real
pi run: the agent found spec 207 by searching the repository, which only worked because the brief happened to mention the
number. A graph author who writes `brief = "Implement the spec."` gets an agent that does not know which spec.

The fix is small and belongs in the node, not in each agent's argv builder: the prompt handed to the agent is the brief plus one
sentence naming the spec path, for BOTH agents (pi and claude) because both are given the same string.

## Requirements
### Functional
- **R1.** A new pure function `pub fn brief_with_spec(brief: &str, spec_path: &str) -> String` in a new file
  `crates/pii-server/src/node_kinds/nodes/coder/prompt.rs`, exported as `pii_server::node_kinds::nodes::coder::prompt::brief_with_spec`
  (add `pub mod prompt;` next to the other `mod` lines in `coder.rs`).
- **R2.** Behaviour of `brief_with_spec`:
  - the result STARTS WITH the original brief, unchanged;
  - it then appends exactly one paragraph: a blank line, then
    ``The spec you implement is `<spec_path>` (relative to your working directory). Read it first: it is the contract.``
  - if `spec_path` is empty (after trimming) the brief is returned unchanged;
  - if the brief already contains `spec_path` as a substring the brief is returned unchanged (never name it twice).
- **R3.** `crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs` hands the agent `brief_with_spec(&brief, &spec.spec)` instead of
  the bare `brief`, at the single place where `plan(...)` is called. Nothing else in `graph_node.rs` changes. In particular the
  empty-brief refusal (`brief_is_usable`) still judges the ORIGINAL brief, not the extended one.

### Non-Functional
- No new dependency. No file over 450 effective lines (`cargo run -q -p xtask -- lint-files`).
- `cargo clippy -p pii-server --all-targets -- -D warnings` clean; the workspace denies `unwrap`/`expect`/`panic` in library code,
  and the new function needs none.

## Architecture
```
brief ──┐
        ├─► brief_with_spec ──► plan(.., &extended_brief, ..) ──► pi argv  /  claude argv
spec  ──┘        (pure)                                          (same string to both)
```

## TDD Contract
New test file `crates/pii-server/tests/coder_brief_names_the_spec.rs` (header: the same
`#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` the neighbouring coder tests use).

| Test name | Input | Expected |
|---|---|---|
| `appends_the_spec_path` | `("Do it.", "specs/9-x.md")` | result contains `` `specs/9-x.md` `` |
| `keeps_the_original_brief_first` | `("Do it.", "specs/9-x.md")` | result `starts_with("Do it.")` |
| `names_the_spec_once_when_the_brief_already_does` (control) | `("Do specs/9-x.md now.", "specs/9-x.md")` | result `== "Do specs/9-x.md now."` |
| `an_empty_spec_path_changes_nothing` (control) | `("Do it.", "  ")` | result `== "Do it."` |
| `a_brief_naming_another_spec_still_gets_this_one` (near miss) | `("See specs/8-y.md.", "specs/9-x.md")` | result contains `` `specs/9-x.md` `` |
| `the_node_hands_both_agents_the_extended_brief` | see below | the agent's received argv contains the spec path |

The last test drives the REAL `CoderFactory` through the real scheduler. Copy the fixture of
`spawned_and_exited_coder_run_succeeds` in `crates/pii-server/tests/coder_run_needs_its_events.rs` (same imports, `root_ctx`,
`run_graph`, `with_criteria` with a `green` = `/bin/true` criterion). Differences: the fake agent script is

```sh
#!/bin/sh
printf '%s\n' "$@" > argv.txt
exit 0
```

(it records every argument it was given AND changes the tree, which the no-change rule of spec 239 requires). Use
`brief = "Implement it."` and `spec = "specs/9-x.md"` in the graph, run it once for `agent = "pi"` (script name `pi`, with a
`provider`-less node so no key is needed) and assert that `argv.txt` in the workdir contains `specs/9-x.md` and that the original
brief `Implement it.` is still there too. A second case with `agent = "claude"` is NOT required: claude needs a credential in the
env, and the extended string is the same for both agents by construction (R3).

## Exit Criteria
- [ ] `cargo:test:pii-server/coder_brief_names_the_spec` (named criterion, resolved by the coder node: it runs
      `cargo test -p pii-server --test coder_brief_names_the_spec` and needs every test above to pass)
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo test -q -p pii-server --test coder_run_needs_its_events 2>&1 | grep -qE "^test result: ok\. [1-9]"` (the neighbour still passes)
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails
- Do NOT change `plan()` in `agent_worker.rs`, `agent_claude.rs`, or any argv builder: both agents already receive whatever string
  the node gives them.
- Do NOT change the `spec` or `brief` fields' meaning, or how the pi session file is named.
- Do NOT edit any existing test file except by adding the new one. Do NOT touch `criteria.rs`.
- Do NOT commit. Leave all work in the working tree.
