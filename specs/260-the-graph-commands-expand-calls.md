# Spec 260: the file-based graph commands expand a `call` like the daemon does

## Overview
**Project**: <repo>
**Phase**: production — composition (spec 255 follow-up, found 2026-10-24 by running the BUILT binary on a real host graph that calls a sub-graph)

Spec 255 added `kind = "call"`, expanded inline at load by `load_host_with_graphs`. Spec 255 R8 says `pii graph check`, run start and resume all see the same expanded graph. Measured with
`<scratch>/compose-real.sh` against the built `pii`: only the daemon path (`pii-server` `lib.rs:486`, `engine/dispatch.rs:366`) and `pii graph explain` (`cmd_graph_installed.rs:268`) expand a call.
The three FILE-based commands do not:

| Command | Loader today (`crates/pii-cli/src/`) | Result on a host with a `call` node |
|---|---|---|
| `pii graph check <file>` | `main.rs:76` `fragment::load_host` | `graph.call.no_dir: a call node needs a graph directory` |
| `pii graph show <file>` | `main.rs:130` `load_toml_with_registry` | `V0: node 'docs' has unknown kind 'call'` |
| `pii graph run <file>` | `cmd_run.rs:164` `load_toml` | not expanded |

So an author cannot check, draw or dry-run a composed graph before installing it, and a typo in a `sha256` pin is only found when the daemon loads it. Spec 255's own tests drove the library, not the binary.

## Verified Premises
- **CONFIRMED** (run, not read) `pii graph explain host --dir <dir>` prints the expanded nodes (`docs__in`, `docs__in__title`, `docs__fmt`) and `pii graph explain bad --dir <dir>` refuses a wrong pin with `fragment.pin.mismatch`. The library works; the three commands above do not call it.
- **CONFIRMED** `load_host_with_graphs(host_src, fragments_dir, graph_dirs, reg)` exists in `crates/pii-graph/src/fragment/weave.rs:105` and is the loader the daemon uses.
- **CONFIRMED** `main.rs:70-73` already derives the fragments dir as `<file's dir>/fragments`; a file's own directory is the natural place to look for the graphs it calls.
- **CONFIRMED** the daemon resolves graphs through `pii_server::graph_search_dirs(graph_dir, store_path)`; `installed_dirs` (`cmd_graph_installed.rs:197`) is the CLI's version of it.
- **INFERRED, MEASURE FIRST** that `cmd_run.rs` `load_and_validate` can take the extra argument without changing `--no-validate` behaviour; read it before editing.

## Requirements
### Functional
- **R1. check.** `pii graph check <file>` loads through `load_host_with_graphs` with the graph dirs `[<the file's directory>]` followed by the installed dirs when they exist. A host that calls a graph in the same directory passes (rc 0, prints `<file>: OK`, or the existing warnings).
- **R2. show.** `pii graph show <file>` draws the EXPANDED graph (the `<call>__<n>` ids appear), through the same loader and dirs as R1.
- **R3. run.** `pii graph run <file>` runs the expanded graph through the same loader and dirs. `--no-validate` keeps its meaning.
- **R4. Same refusals as the daemon.** A wrong `sha256` prints the `fragment.pin.mismatch` error naming the expected and the actual digest, and exits non-zero. A call to a graph that is not in any dir prints `graph.call.not_found`. A graph that calls itself prints the chain `a -> a`. Each exits non-zero and runs NOTHING.
- **R5. One helper.** The dirs list is built in ONE function used by all three commands (new file `crates/pii-cli/src/graph_dirs.rs`); no command builds its own.
- **R6. No behaviour change without a call.** A graph with no `call` node behaves exactly as before for all three commands; every existing CLI test passes unedited.

### Non-Functional
- No new dependency, no new raw print outside the existing seams (the observability budget for `main.rs` is already at its limit: reuse the existing `fail(...)` and `println!` sites, add none). Files stay under 450 effective lines: if `main.rs` would pass it, the helper and the three call sites' bodies move into `graph_dirs.rs`.
- Tests use temp dirs and the BUILT binary (`env!("CARGO_BIN_EXE_pii")`); no absolute path, no network, no daemon.

## TDD Contract
New file `crates/pii-cli/tests/graph_commands_expand_calls.rs`. Each test writes a host and a `note` sub-graph (one `transform` node, `exit` declared) into a temp dir, computes the sub-graph's sha256 in the test, and RUNS the binary.

| Test name | Setup | Expected |
|---|---|---|
| `check_passes_a_host_that_calls_a_graph` | host calls `note`, correct pin | rc 0 |
| `check_refuses_a_wrong_pin_naming_both_digests` | pin of 64 zeros | rc != 0; stderr contains `pin` and the real digest's first 12 hex |
| `check_refuses_a_missing_graph` | call names `nope` | rc != 0; stderr contains `not_found` |
| `check_refuses_a_graph_that_calls_itself` | `a` calls `a`, with `a`'s real digest | rc != 0; stderr contains `a -> a` |
| `show_draws_the_expanded_ids` | correct host | rc 0; stdout contains `docs__in` and `docs__fmt` |
| `run_executes_the_expanded_graph` | host maps `title` from `$.input.spec`; `--input '{"spec":"hello"}'` | rc 0; the run's output contains `hello` (read from the printed result) |
| `run_with_a_wrong_pin_runs_nothing` | pin of 64 zeros | rc != 0; no store file is created at the `--db` path |
| `a_graph_without_a_call_is_unchanged` | the tracked `graphs/queue.toml`-style host with no call (use a 2-node host) | check, show and run behave as before (CONTROL: nothing changed for existing graphs) |
| `a_call_to_a_graph_in_another_directory_is_not_found` | `note` lives in a sibling directory, not the file's own | rc != 0 (CONTROL: the lookup is the file's directory, not a global search) |

**The requirement a plausible implementation satisfies only in appearance:** fixing `check` alone (the command the spec named) and leaving `show` and `run` on the old loaders. `show_draws_the_expanded_ids` and `run_executes_the_expanded_graph` are the tests that catch it; each drives a different command.

## Exit Criteria
- [ ] `cargo test -q -p pii-cli --test graph_commands_expand_calls 2>&1 | grep -qE "^test result: ok\. [9] passed"`
- [ ] `test -f crates/pii-cli/src/graph_dirs.rs`
- [ ] `cargo test -q -p pii-graph --test call_inlines_a_graph 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-cli 2>&1 | grep -v "^test result: ok" | grep -c "test result: FAILED" | grep -qx 0`
- [ ] `cargo clippy -q -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `cargo run -q -p xtask -- lint-observability > /dev/null 2>&1`

## Guardrails
- Do NOT change `load_host_with_graphs`, the `call` expansion, or any file under `crates/pii-graph/src/fragment/`: this spec routes three commands to it.
- Do NOT edit any existing test; a failing existing test is a finding to report.
- Do NOT add a raw print macro; do NOT raise the observability budget.
- Do NOT commit.

## Follow-ups (not this spec)
The call pins in the run record and `run explain`; a live run of a composed graph through the daemon with a real agent node; the documentation sub-graph itself.
