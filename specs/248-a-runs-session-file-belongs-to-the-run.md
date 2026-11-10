# Spec 248: a run's session file belongs to the run

## Overview
**Project**: <repo>
**Phase**: post-vision hardening — agent management (defect 19 in `_docs/05-operations/agent-management-findings-2026-10-23.md`)

Two coder runs pointed at one workdir share ONE pi session file. The default is computed twice, from the spec's file stem
only — `agent_worker::pi_session_path` (`{workdir}/.pii-sessions/{stem}.jsonl`) and the same expression again inside
`plan()` for the pi argv — so run A and run B both name `…-sessions/245-a-commit-builds-at-its-own-head.jsonl`.
Measured 2026-10-23: two runs at once on one workdir, a single 204,737-byte session file carrying both conversations, and
nothing in either run's journal saying the file was shared. A resumed session must keep working, so the fix cannot simply
make every path unique: an explicit `session =` in node data stays the operator's deliberate pin.

What is wrong is the DEFAULT, not the possibility of sharing: a run that did not ask to share must get a file of its own.

## Requirements
### Functional
- **R1.** `WorkerSpec` (in `crates/pii-server/src/node_kinds/nodes/agent_worker.rs`) gains `pub run: Option<String>` — the
  run whose session this dispatch writes. `WorkerSpec::from_cfg` reads an optional `run` string; absent is `None`.
- **R2.** `pi_session_path(spec)`'s default carries the run id when it is set: `{workdir}/.pii-sessions/{stem}-{run}.jsonl`.
  With no `run` the path is the shape it is today (`{stem}.jsonl`) — the pure function keeps one contract and one call site
  computes it. An explicit `session` still wins, unchanged, and is returned exactly as written.
- **R3.** The coder node sets `run` from `ctx.trace().run` on the `WorkerSpec` before it plans anything, so the pi argv
  (`plan`) and the pre-spawn header (`paths::prepare_pi_session`) name the SAME file. The container form inherits the field
  through the existing `WorkerSpec { workdir: CONTAINER_WORKDIR, ..spec.clone() }` update, so the `/work` argv carries the
  same run id.
- **R4.** A `run` value that is not `[A-Za-z0-9._-]+`, or is empty, is refused by `from_cfg` with a message naming the field,
  before any spawn: a graph-supplied handle must not be able to escape `.pii-sessions/`.
- **R5.** `coder.spawned.session` (already journalled from the argv by `session_of`) names the run-scoped path, and that file
  exists on disk with a pi header before the child starts. Two runs on one workdir therefore journal two different paths and
  neither file receives the other run's bytes.
- **R6.** An explicit `session = "<abs path>"` in node data is honoured for every run that names it: two runs with the same
  explicit path share it (resume across runs). R2's run-scoping applies to the default only.

### Non-Functional
- `crates/pii-server/src/node_kinds/nodes/agent_worker.rs` is **388 effective lines** (measured: `cargo run -q -p xtask --
  lint-files`); the limit is 450 and the warn line is 380. The field, its parse and its doc comment fit in that headroom —
  keep the doc comments tight and keep the change inside that file. Do NOT create a new module for it: the commit path list
  is fixed, and a new file nobody staged is how a green item lands half-committed. If the file would cross 450, STOP and say
  so in your final message.
- No file over 450 effective lines. clippy clean (`-D warnings`), no `unwrap`/`expect`/`panic` in library code.

## Architecture
```
graph_node.rs  let spec = WorkerSpec { run: Some(ctx.trace().run.as_str().to_owned()), ..spec };
                    |
                    +-- plan(&planned, …)            -> pi argv  --session <path>
                    +-- launch_on_host(…, &spec, …)  -> prepare_pi_session(&spec) -> header at <path>
                                        pi_session_path(spec)            (single source)
```
`pi_session_path` stays the only place that builds a default. `plan` calls it instead of repeating the `format!`. Nothing
else changes in the pii argv: `--mode json`, the provider flag, the at-rest key lookup and `-p <brief>` are as today.

Rejected: a per-run random directory (breaks the doc's own rule that a dispatch resumes a named file, and hides the path from
the journal); a lock file beside the shared session (the file is still shared — the defect is that two conversations land in
one file); keying on the pid (a resumed session is a different pid, so the handle would move).

## TDD Contract
New test file `crates/pii-server/tests/coder_session_is_per_run.rs`. Copy the header, `Rec` observer, `fake_bin`,
`root_ctx` and `criteria_table` from `crates/pii-server/tests/coder_spawns_a_process.rs` (same crate). The stand-in pi must
**write into the session file it is handed** — that is what makes the defect visible:
`while [ $# -gt 0 ]; do case "$1" in --session) S="$2"; shift;; esac; shift; done; echo "$$" >> "$S"`.

The helper builds ONE store and takes TWO run ids from it (`store.next_run_id()` twice — two stores would hand out the same
`r000000000001` and hide the defect), then runs the same one-node coder graph twice, sequentially, against ONE workdir.

| Test name | Input | Expected |
|---|---|---|
| `two_runs_on_one_workdir_get_two_session_files` | two runs, same workdir, same spec path, no `session =` | the two `coder.spawned.session` values differ AND both name existing files; each file holds exactly one appended pid line; run A's bytes are identical after run B ran |
| `the_journalled_session_is_the_file_on_disk` | one run | the path read from `coder.spawned.session` exists and starts with the pi header written before the spawn (`{"type":"session"`) |
| `an_explicit_session_pins_the_path` (control) | two runs with `session = "<workdir>/pinned.jsonl"` | both journal that same path, and the file holds BOTH pids: R2 does not override a deliberate pin |
| `the_default_session_path_carries_the_run` (unit) | `from_cfg` with `run = "r000000000042"` | `pi_session_path` ends `…/.pii-sessions/<stem>-r000000000042.jsonl`; with no `run` it ends `…/<stem>.jsonl` |
| `a_malformed_run_is_refused` | `run = "../../etc/x"` and `run = ""` | `from_cfg` is an error whose message names `run`, and no path is produced |

`crates/pii-server/tests/agent_session_header.rs` has four tests today, one of which
(`the_default_session_path_follows_the_spec_stem`) asserts the old default. Update that test to the R2 rule and keep all
four: the count must not drop.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_session_is_per_run 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test agent_session_header 2>&1 | grep -qE "^test result: ok\. 4"`
- [ ] `cargo test -q -p pii-server --test coder_spawns_a_process 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -qE "^test result: ok\. [1-9]"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails
- Do NOT change the claude arm (`agent_claude`, spec 244), the quota fields (237), the tree digest (217), or `session_of`.
- Do NOT weaken or delete an existing test. `agent_session_header` keeps four tests; `coder_spawns_a_process`,
  `agent_claude_session_handle` and `agent_provider_on_argv` must pass unedited. If one fails for a reason other than the
  default's shape, STOP and report which test and why instead of editing it.
- Do NOT make the path unique by adding randomness to the file NAME: the journal is what resumes a run, and a path nothing
  can predict cannot be resumed.
- Do NOT move scratch outside `_tmp` and do NOT write to `/tmp`.
- If the premise above does not match the code you find (e.g. `pi_session_path` is already run-scoped, or the coder no longer
  plans a session), STOP and say so in your final message instead of forcing the change.
- Do NOT commit. Leave all work in the working tree.

## Closure
Owed after the commit: the two-run reading on the committed tree (two ids from one store, two distinct files, 0 bytes of one
in the other), the mutation that must redden `two_runs_on_one_workdir_get_two_session_files` (drop the run id from the path,
restore with `touch` + `sha256sum` check, never `cp -p`), and the `lint-files` count for `agent_worker.rs` afterwards.
