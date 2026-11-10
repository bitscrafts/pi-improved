# Spec 259: the judge reads a spec's own Exit Criteria from its committed blob — the generic graph gains a verdict

## Overview
**Project**: <repo>
**Phase**: production — generic spec graph, slice 2 (operator, 2026-10-24: every spec is implemented by pii; Opus design review `<scratch>/pii-graph-design/pii-graph-design-plan-20261024T155725Z.md`, Q2 and Verdict 3)

After spec 257 the tracked graph `graphs/spec.toml` runs one agent on one spec, but its verdict is only "the agent exited 0". The criteria that decide whether the spec is met live in the spec
markdown (`- [ ] \`cmd\``), copied into `scripts/autonomy/plan.tsv` (a second source of truth), and run by the shell loop afterwards. The existing `judge` node (`crates/pii-server/src/node_kinds/nodes/queue.rs`)
runs an item's criteria through `/bin/sh -c` and stops at the first failure, but it takes them from a queue-store item that no production code writes.

This spec gives `judge` a second mode: given a spec path and a commit id, read the spec's Exit Criteria from that commit's blob, run EVERY one, and report each result. The criteria text never comes
from the working tree, the run input or the agent: a worker can edit its own spec, and must gain nothing by it. The graph gets a `verify` judge after the coder and a refusal that says the criteria failed.

## Verified Premises
- **CONFIRMED** `JudgeNode` (`queue.rs:112-165`) reads `item.criteria` from `nodes.select.item`, runs each as `["/bin/sh","-c",command]` through `ctx.proc()` in `self.workdir` with `CRITERION_TIMEOUT` (10 minutes, `queue.rs:21`), and stops at the first non-zero. `JudgeFactory::build` requires a literal `workdir` string; ports are `ok` and `fail`.
- **CONFIRMED** the node's process scope is `["/bin/sh"]` plus agent binaries, podman and the criteria programs (`agent_binaries`, `run_programs`); `git` is NOT a permitted argv0, so the blob is read through `/bin/sh` with positional arguments, never by spawning `git` directly and never by interpolating text into a command string.
- **CONFIRMED** `pii_graph::expr::eval_or_literal` resolves `$.input.<path>` and an unresolvable path is `Null` (`expr.rs`); spec 257's `coder/input_cfg.rs` is the pattern. The judge must resolve its own keys at RUN time (its `build` runs before there is an input).
- **CONFIRMED** `refuse` (`crates/pii-graph/src/nodes.rs:240`) takes a free-string `hook` and `reason`, so a second refusal can say the criteria failed.
- **CONFIRMED (measured by the Opus review)** 1,453 criterion lines exist in `specs/`; about 570 contain a shell operator (`| && ; $( >`). They cannot run as a bare argv, so the shell is kept, and the trust boundary is the committed blob.
- **CONFIRMED** the criteria grammar: a line `- [ ] ` followed by a backticked span; the command is the FIRST backticked span of the line (the `spec-author` skill's rule, also what `scripts/autonomy/plan_import.py` reads).

## Requirements
### Functional
- **R1. Mode.** `judge` with a `spec` key in its config runs in spec mode. Without `spec` it behaves exactly as today (item mode): every existing judge test passes unedited.
- **R2. Keys.** In spec mode the keys `spec`, `base` and `workdir` are resolved at run time with `eval_or_literal` against `{"input","payload","nodes"}`. `JudgeFactory::build` no longer requires `workdir` to be present when `spec` is; item mode still requires it.
- **R3. The base must be a commit id.** `base` must match `^[0-9a-f]{40}$`. Anything else (`HEAD`, a branch name, a short id, an empty string, `Null`) ends on the `fail` port with the reason `judge: base must be a 40-hex commit id, got '<value>'` and NOTHING is run.
- **R4. The blob.** The spec text is read with `/bin/sh -c 'git show "$1:$2"' sh <base> <spec>` run in `workdir`. A non-zero exit ends on `fail` with `judge: cannot read <spec> at <base>` plus the first line of stderr, and nothing is run. The working-tree copy of the spec is never read.
- **R5. Parsing.** The criteria are the first backticked span of every line that starts `- [ ] ` or `- [x] ` inside the section that begins at the line `## Exit Criteria` and ends before the next line starting `## ` (or at end of file). A spec with no such section, or an empty one, ends on `fail` with `judge: <spec> has no Exit Criteria`.
- **R6. Run all.** Every criterion runs, in order, through `["/bin/sh","-c",<command>]` in `workdir` with `CRITERION_TIMEOUT`, and a failure does NOT stop the rest: the point is the whole list (a worker's single-criterion pass missed 35 broken tests in spec 247).
- **R7. The verdict.** Port `ok` only when every criterion exits 0. Both ports carry `{"verdict":"passed"|"failed","spec":<path>,"base":<id>,"criteria":<count>,"passed":<count>,"results":[{"criterion":<command>,"rc":<int>}...]}`; `fail` adds `"reason":"criterion failed: <first failing command> (rc N)"` and `"failing":[<every failing command>]`.
- **R8. The graph.** `graphs/spec.toml` gains a node `verify` of kind `judge` with `spec = "$.input.spec"`, `base = "$.input.base"`, `workdir = "$.input.workdir"`, `caps = ["proc"]`; the edge `code --ok--> done` is replaced by `code --ok--> verify`, `verify --ok--> done`, `verify --fail--> unverified`, where `unverified` is a new `refuse` node (`hook = "judge"`, `reason = "the spec's criteria failed"`). `code --err--> refused` is unchanged. The file keeps no absolute path and no credential.

### Non-Functional
- No new dependency. Files stay under 450 effective lines: put spec mode in a new `crates/pii-server/src/node_kinds/nodes/judge_spec.rs`; do not grow `queue.rs` by more than 25 lines.
- Python only through uv (this spec needs none). No hardcoded path; the judge runs wherever `workdir` points.
- The criteria's own output is not journalled by this spec (only rc and the command); no event is added.

## Architecture
```
run input {spec, base, workdir, ...} -> coder (ok) -> verify (judge, spec mode) --ok--> done
                                                              |  1. base = 40 hex?              (R3)
                                                              |  2. sh -c 'git show "$1:$2"' sh base spec   (R4: the blob, never the tree)
                                                              |  3. parse "## Exit Criteria" -> commands      (R5)
                                                              |  4. run ALL via sh -c, collect rc             (R6)
                                                              '--fail--> unverified (refuse: "the spec's criteria failed")
```

## TDD Contract
New file `crates/pii-server/tests/judge_reads_the_spec_blob.rs`. Each test builds a throwaway git repo in a temp dir (git init, one commit holding a spec file and, where needed, a marker file), then RUNS the node. Names are the contract.

| Test name | Setup | Expected |
|---|---|---|
| `every_criterion_of_the_blob_runs_and_all_pass` | spec with 3 criteria that exit 0 | port `ok`, `passed` = 3 of 3, `results` has 3 entries |
| `a_failure_does_not_stop_the_rest` | criteria: pass, FAIL, pass | port `fail`, `results` has 3 entries (the third ran), `failing` lists exactly the middle one |
| `the_working_tree_spec_is_never_read` | blob has a FAILING criterion; the working-tree copy is edited to pass | port `fail` (the blob wins) |
| `an_edit_that_adds_a_criterion_is_ignored` | working tree adds a failing criterion the blob lacks | port `ok` with the blob's count |
| `a_non_commit_base_is_refused_and_nothing_runs` | `base` = `HEAD`, then a 7-hex id, then empty | each ends on `fail` naming `base`; a marker-writing criterion's file does NOT exist |
| `a_real_40_hex_base_is_accepted` | `base` = the repo's real commit id | runs (CONTROL for the refusal above) |
| `an_unknown_spec_path_fails_naming_it` | `spec` = `specs/nope.md` | `fail`, reason names the path and the base; nothing runs |
| `a_spec_without_exit_criteria_fails` | spec with no `## Exit Criteria` | `fail`, reason says so |
| `the_section_ends_at_the_next_heading` | a backticked checkbox line AFTER `## Guardrails` | it is not run |
| `a_checked_box_is_a_criterion_too` | `- [x] \`cmd\`` | it runs |
| `only_the_first_backticked_span_is_the_command` | a line with two backticked spans | the first runs, the second does not |
| `input_keys_resolve_from_the_run_input` | `spec`, `base`, `workdir` as `$.input.*` | the node reads them from the input |
| `item_mode_is_untouched` | the existing judge config with no `spec` | behaves as before (CONTROL: the new mode changes nothing for existing graphs) |
| `a_criterion_text_cannot_inject_through_the_arguments` | `spec` containing `"; touch marker; "` | the read fails; the marker file does NOT exist |

New file `crates/pii-graph/tests/spec_graph_has_a_verdict.rs`:

| Test name | Expected |
|---|---|
| `the_graph_routes_ok_through_verify_to_done` | the loaded graph has edges `code.ok -> verify`, `verify.ok -> done`, `verify.fail -> unverified`, `code.err -> refused`, and NO edge `code.ok -> done` |
| `a_failed_verdict_cannot_reach_done` | `unverified` is a `refuse` node with no outgoing edge |
| `the_graph_still_validates_with_no_absolute_path` | `validate` has no errors and no string value starts with `/` |

**The requirement a plausible implementation satisfies only in appearance:** reading the spec from the working tree because it is simpler (`std::fs::read_to_string(workdir.join(spec))`). Every happy-path test passes, and a worker that rewrites its own criteria certifies itself. `the_working_tree_spec_is_never_read` and `an_edit_that_adds_a_criterion_is_ignored` are the tests that catch it; both must run the node, and the repo's working tree must really differ from the blob.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test judge_reads_the_spec_blob 2>&1 | grep -qE "^test result: ok\. (1[4-9]|[2-9][0-9]) passed"`
- [ ] `cargo test -q -p pii-graph --test spec_graph_has_a_verdict 2>&1 | grep -qE "^test result: ok\. [3-9] passed"`
- [ ] `cargo test -q -p pii-graph --test spec_graph_is_tracked_and_valid 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --test queue_graph_pipeline 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -qE "^test result: ok"`
- [ ] `test -f crates/pii-server/src/node_kinds/nodes/judge_spec.rs`
- [ ] `cargo clippy -q -p pii-server -p pii-graph --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT read the spec from the working tree, and do NOT take criteria text from the run input, the agent output or the queue store in spec mode.
- Do NOT spawn `git` as an argv0 directly, and do NOT build a command string from `spec` or `base`: they are positional arguments of a fixed script.
- Do NOT change item mode, the existing ports, or an existing event or journal field.
- Do NOT edit any existing test; a failing existing test is a finding to report.
- Do NOT put an absolute path or a credential in `graphs/spec.toml`.
- Do NOT commit.

## Follow-ups (not this spec)
Mutation (revert the non-test hunks, the targets must go red); `red` (the same list on the unchanged tree, partitioned into targets and guards); classified failure ports; the bounded fix loop with a port when its budget runs out; a charge of the coder's journalled spend to the run budget; `qgate`; the documentation sub-graph through the `call` node; the lane script moving onto `pii run start spec --input`.
