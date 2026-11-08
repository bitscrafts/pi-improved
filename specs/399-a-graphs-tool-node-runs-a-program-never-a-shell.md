# Spec: a graph's `tool` node runs a program, never a shell

## Overview
Spec 393 R3 declared that `tool`'s surface is closed: `fs_read`, `glob`, `grep`, `fs_write`, `run` with a
literal argv0 and no shell. Only half of it landed. The load check (`check_tool_surface`,
`crates/pii-graph/src/nodes.rs`) refuses a `run = "..."` key, but `ToolNode` never executes `run`, and every
shipped `tool` node still says `tool = "bash"`, which runs `/bin/sh -c <string>`
(`crates/pii-tools/src/bash.rs`). Measured 2026-11-07: six such nodes in three graphs.

```
graphs/books.toml:20            args = { command = "$.input.inventory", cwd = "$.input.workdir" }
graphs/books.toml:59            args = { command = "$.input.verify",    cwd = "$.input.workdir" }
graphs/starter/heal-fix.toml:22 args = { command = "$.input.test_command", ... }   (and :65)
graphs/starter/heal-fix.toml:87 args = { command = "git diff", ... }
graphs/starter/verify-fix.toml:18 args = { command = "$.input.test_command", ... }
```

The command is a string that arrives as run input, so whoever starts the run chooses what the shell executes.
The fence (spec 393) says authority comes from the committed spec, not from the graph or its input. This
row makes that true for `tool` by adding the `run` builtin tool (an argv array, executed without a shell)
and making `tool = "bash"` a load-time refusal.

**Project**: /projects/pi-improved
**Phase**: 383 follow-up (393 R3, second half)
**Depends on**: 393

## Requirements
### Functional
- R1 a builtin tool `run` in `crates/pii-tools` takes `{ argv: [string, ...], cwd?, timeout_ms? }`. It executes
  `argv` exactly, through the same `ctx.proc()` capability and the same workspace-bounded `cwd`, output cap and
  timeout bounds as `bash`. Nothing is joined into a string and nothing reaches `/bin/sh -c`.
- R2 `run` refuses, at call time and naming the program, an argv whose first entry is a shell (`sh`, `bash`,
  `dash`, `zsh`, `ksh`, `fish`, `env`, matched on the final path component, the same list
  `check_tool_surface` uses) or is empty. The check is on the RESOLVED argv, because the argv arrives as run
  input: the load check cannot see it.
- R3 the graph loader refuses a `tool` node whose `tool` is `bash`, with a message that names the node and says
  to use `run`. The agent-facing `bash` tool in the coder toolset is a different surface and is NOT changed.
- R4 the shipped graphs move to `run`: `books` takes `inventory` and `verify` as argv arrays; `doc-code` supplies
  them as literal arrays (`["cargo","xtask","doc-drift-inventory"]`, `["cargo","xtask","doc-drift-check","--modules"]`);
  `heal-fix` and `verify-fix` take `test_argv` (an array) instead of `test_command` (a string); `heal-fix`'s
  `git diff` becomes `["git","diff"]`. The `needs` lists, the run-input documentation in
  `crates/pii-server/src/lib.rs` / `engine/runs.rs` / `cmd_oneshot.rs`, and the tests move with them.
- R5 `graphs/` contains no `tool = "bash"`, and `docs/03-reference/graph-vocabulary.md` no longer says the
  surface is still open.

### Non-Functional
- R6 pinned graphs stay pinned: `doc-code` calls `books` by sha256, so `bash scripts/repin-graphs.sh` is part of
  the change and its result is committed.
- R7 no new dependency and no file over 450 effective lines.

## Architecture
```
graph file (committed)        run input (data)                 execution
tool = "run"           +      argv = ["cargo","test",...]  ->  ToolNode -> pii_tools::run
args = { argv = "$.input.x" }                                    |- argv0 is a shell?  -> Err(Denied), names program
                                                                 |- else ctx.proc().run(argv, cwd, timeout)  (no /bin/sh)
tool = "bash"  ->  refused at load: tool.bash.shell: node 'N' ...
```
Key decisions:
- The refusal of a shell program is at RUN time on the resolved argv, not only at load. The load check cannot
  see `$.input.x`; checking only there is the shape that looks closed and is not (the same class as a check
  fed a source constant instead of the delivered message).
- An argv from input is accepted, a string is not. Rejected: splitting a string on whitespace. It looks
  equivalent and re-opens quoting (`"a b"`), globbing and `;` in a string a caller still thinks is a command line.
- `bash` stays registered for the coder's agent toolset (an agent with a shell is a separate, bounded decision,
  spec 215); only the graph `tool` node refuses it.

## TDD Contract
| Test name | Given | Expects |
|---|---|---|
| `run_executes_an_argv_without_a_shell` | argv `["echo","a;b"]` | stdout is `a;b` (a shell would split on `;`) |
| `CONTROL_run_passes_arguments_with_spaces_whole` | argv `["printf","%s","x y"]` | stdout `x y`, one argument |
| `a_shell_argv0_is_refused_even_when_it_arrives_as_data` | argv `["sh","-c","true"]`, `["/bin/bash","-c","x"]`, `["env","sh"]` | `Denied`, message names the program, nothing ran |
| `an_empty_argv_is_refused` | argv `[]` | `Invalid` |
| `CONTROL_a_program_named_like_a_shell_prefix_is_not_a_shell` | argv `["shfmt","--version"]` is attempted | NOT refused as a shell (the match is on the whole component) |
| `a_tool_node_naming_bash_is_refused` | `tool` node with `tool = "bash"` | load error `tool.bash.shell`, names the node |
| `CONTROL_a_tool_node_naming_run_loads` | `tool` node with `tool = "run"` | loads |
| `books_runs_its_inventory_as_an_argv` | books with `inventory = ["true"]`, `verify = ["true"]` | the graph runs to its exit |
| `CONTROL_books_still_ends_on_nothing_to_do_for_an_empty_inventory` | inventory prints nothing | `nothing-to-do`, as before |

Acceptance test: `a_shell_argv0_is_refused_even_when_it_arrives_as_data`. The requirement a plausible
implementation satisfies in appearance only is R2: a load-time check passes every shipped graph while an
argv supplied through `--input` still reaches a shell. The test drives the node with the argv as RUN INPUT.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change.
- [ ] `cargo test -q -p pii-tools --test run_tool 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-graph --test tool_surface_is_closed a_tool_node_naming_bash_is_refused 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -d graphs && ! grep -rq 'tool = "bash"' graphs/`
- [ ] `cargo test -q -p pii-server --test books_graph books_runs_its_inventory_as_an_argv 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test books_graph CONTROL_books_still_ends_on_nothing_to_do 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -f docs/03-reference/graph-vocabulary.md && ! grep -q 'still reaches a shell' docs/03-reference/graph-vocabulary.md`

### Carried re-checks
These pass before this row; their passing is not evidence it is done.
- [ ] `cargo test -q -p pii-graph --test tool_surface_is_closed 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo run -q -p xtask -- lint-graph-kinds 2>&1 | grep -q '0 specialised'`

## Guardrails
- Do NOT split a string into an argv, anywhere. A string is refused, not interpreted.
- Do NOT remove or change the coder's agent-facing `bash` tool or its deny list.
- Do NOT add an exception to the shell list to make a graph pass; move the graph.
- Do NOT edit a pinned graph without re-pinning it (`bash scripts/repin-graphs.sh`) and committing the result.
- Do NOT weaken the existing `tool_surface_is_closed` tests.

## Files
- `crates/pii-tools/src/run.rs`
- `crates/pii-tools/src/lib.rs`
- `crates/pii-tools/tests/run_tool.rs`
- `crates/pii-graph/src/nodes.rs`
- `crates/pii-graph/tests/tool_surface_is_closed.rs`
- `crates/pii-server/src/lib.rs`
- `crates/pii-server/src/engine/runs.rs`
- `crates/pii-cli/src/cmd_oneshot.rs`
- `crates/pii-server/tests/books_graph.rs`
- `crates/pii-server/tests/doc_code_graph.rs`
- `graphs/books.toml`
- `graphs/doc-code.toml`
- `graphs/starter/heal-fix.toml`
- `graphs/starter/verify-fix.toml`
- `graphs/README.md`
- `docs/03-reference/graph-vocabulary.md`
