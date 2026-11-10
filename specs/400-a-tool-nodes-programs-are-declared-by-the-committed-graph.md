# Spec: a `tool` node's programs are declared by the committed graph, never by run input

## Overview
Row 399 made `tool` run a program, not a shell, and required every `run` node to declare the programs it may exec.
Review of that row (2026-11-09) measured what it still allows. The declaration may be an expression, so three shipped graphs
take it from run input:

```
graphs/books.toml:21               programs = "$.input.programs"
graphs/starter/heal-fix.toml:23,67 programs = "$.input.test_programs"
graphs/starter/verify-fix.toml:19  programs = "$.input.test_programs"
```

Measured through the real tool node with the program declared: `["python3"]` ran `python3 -c ...` and exited 0 (uid 0);
`["nice"]` ran `nice sh -c "echo WRAPPED"` and printed `WRAPPED`; `["sh"]` was refused. The shell refusal is a blacklist on the
program's name, so any declared program that executes another command is a shell. The only thing that keeps that bounded is who
writes the declaration, and for these three graphs that is whoever starts the run.

The fence (spec 393) says authority comes from the committed file, not from run input. This row makes that true for `programs`:
it is a literal list in the graph file, and a run input can supply arguments but never the program.

**Project**: <repo>
**Phase**: 383 follow-up (399, the programs half)
**Depends on**: 399

## Requirements
- R1 `check_tool_surface` refuses, at load and naming the node, a `tool = "run"` node whose `programs` is not an array of
  literal strings: a `$.` expression, a string, a table, an empty array, or an empty or whitespace entry is refused.
- R2 it also refuses a `programs` entry whose final path component is a shell (the one `SHELLS` constant), so a graph file cannot
  declare `sh`, `bash` or `env`.
- R3 `ToolFactory::declared_programs` no longer evaluates an expression: a malformed `programs` declares nothing (it already fails
  closed) and the load check makes the case unreachable for a loaded graph.
- R4 `books` declares its program in the file. Its callers pass `inventory` and `verify` argv arrays; the `programs` input is
  removed from `books`' `needs`. `doc-code` stays `["cargo"]` and is re-pinned to the new `books` digest.
- R5 `starter/heal-fix` and `starter/verify-fix` declare the test program literally (`["cargo"]`), as the shipped seeded test uses
  `cargo`; their `test_programs` input is removed and the comments that name it are updated.
- R6 `graphs/` contains no `programs = "$..."`. A control proves a literal declaration still loads and a call still runs.
- R7 `docs/03-reference/graph-vocabulary.md` states the rule and names the limit that stays: a declared program that itself runs
  commands (an interpreter) is the committed file's decision, reviewed with the graph.

## Architecture
```
graph file (committed)  programs = ["cargo"]  ->  load check R1/R2  ->  run scope admits exactly those names
run input               argv = ["cargo","test"]                      ->  argv0 must EQUAL a declared name (spec 399 R8)
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `a_programs_expression_is_refused_at_load` | `programs = "$.input.programs"` | load error naming the node and `programs` |
| `a_programs_array_of_literals_loads` (CONTROL) | `programs = ["cargo"]` | loads |
| `a_declared_shell_is_refused_at_load` | `programs = ["sh"]`, `["/bin/bash"]`, `["env"]` | load error naming the node and "shell" |
| `a_declared_program_named_like_a_shell_prefix_loads` (CONTROL) | `programs = ["shfmt"]` | loads |
| `an_empty_or_blank_programs_is_refused` | `[]`, `[""]`, `["  "]` | load error |
| `books_declares_its_program_in_the_file` | the shipped `books.toml` | its `run` nodes carry literal `programs`; `needs` has no `programs` |
| `CONTROL_books_still_runs_an_inventory_argv` | books with `inventory = ["true"]` | runs to its exit as before |
| `run_input_cannot_widen_the_programs` | books run with a hostile `programs = ["python3"]` input and `inventory = ["python3","-c","print(1)"]` | the node is denied; nothing ran |

Acceptance test: `run_input_cannot_widen_the_programs`. The requirement a plausible implementation satisfies in appearance only
is R1: a load check that looks at the shipped graphs passes while an expression still resolves at run time. The test drives the
hostile value as RUN INPUT.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that passes today is a Guardrail, never a criterion here.
- [ ] `cargo test -q -p pii-graph --test tool_programs_are_literal 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test books_graph books_declares_its_program_in_the_file 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test books_graph run_input_cannot_widen_the_programs 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -d graphs && ! grep -rqE 'programs *= *"[$]' graphs/`
- [ ] `test -f docs/03-reference/graph-vocabulary.md && grep -q 'programs.*literal' docs/03-reference/graph-vocabulary.md`

## Guardrails
- Keep these green; they pass today, so they are not criteria: `cargo test -q -p pii-server --test tool_programs_are_declared`,
  `cargo test -q -p pii-graph --test tool_surface_is_closed`, `cargo run -q -p xtask -- lint-graph-kinds` (prints `0 specialised`).
- Do NOT add a program to the shell list to make a graph pass, and do NOT keep an escape hatch that evaluates `programs` at run time.
- Do NOT widen the run scope for any kind other than a `tool = "run"` node's own declared names.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-graph/src/nodes.rs`
- `crates/pii-graph/tests/tool_programs_are_literal.rs`
- `crates/pii-graph/tests/tool_surface_is_closed.rs`
- `crates/pii-server/src/engine/tool.rs`
- `crates/pii-server/tests/tool_programs_are_declared.rs`
- `crates/pii-server/tests/books_graph.rs`
- `crates/pii-server/tests/doc_code_graph.rs`
- `crates/pii-server/tests/slice_e_live.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-cli/src/cmd_oneshot.rs`
- `graphs/books.toml`
- `graphs/doc-code.toml`
- `graphs/starter/heal-fix.toml`
- `graphs/starter/verify-fix.toml`
- `graphs/README.md`
- `docs/03-reference/graph-vocabulary.md`
