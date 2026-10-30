# Spec 317: `pii lane plan` — the composed lane's whole setup half (`run-batchC-pii.sh` lines 29-218), ported into the binary

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the product must not need companion scripts (user, 2026-10-29: "the final project doesn't have companion scripts")

The composed lane that runs the queue is `scripts/run-batchC-pii.sh` (343 lines). Nearly every defect found while exercising composability on 2026-10-28/29 was in that layer (the session lookup, the pin check, the input keys, `python3` missing in the judge), not in the engine. This is part 1 of two. Part 1 ports **everything the script does before it starts a run**, lines 29-218, in both modes (dry and real): arguments, the dirty check, lane choice, the brief, the coder-lane graph or the spec-lane input, the HEAD and pin checks, the deadline, the resume lookup, the container preflight, the printed block and the log header. The two probes that touch the devbox (the resume lookup at lines 101-107 and the `uv` preflight at lines 211-216) sit behind a `Devbox` trait: this spec ships a dry implementation and a test stub, and part 2 ships the ssh implementation together with start, poll, facts, quota text and the spec guard (lines 219-343), then retires the script.

`pii lane plan <N> [--resume <sid>] [<extra-brief-file>] [--dry-run]` reproduces the observable behaviour of `scripts/run-batchC-pii.sh <N> [...] --dry-run`: stdout, stderr streams, exit code and every file written, in the same order. The script is the reference. It is **not edited by this spec**. Named divergences are listed in the last section; there are no others.

`S:<n>` below means line n of `scripts/run-batchC-pii.sh` at the commit that installs this spec.

## Requirements
### Functional
- R1 (the two roots): `plan` takes `--root <dir>`, defaulting to the git top level of the current directory. There are two values, `root` (where files are read and written) and `agent_root` (the path the devbox sees, written into the input's `workdir`, the brief's `$LOGD` and the preflight's `-w`); `agent_root` defaults to `root` and is overridden with `--agent-root`. No path is hardcoded (the script hardcodes it at `S:21`). The script `cd`s to its root whatever the caller's directory; so does `plan`, and a git top level that lies inside `_tmp/` is refused.
- R2 (arguments and order of checks). In this order, each with the script's exit code and stream: `mkdir -p _tmp/claude-batchC _tmp/devbox-test/graphs` first, before any validation (`S:30`); no spec number → the usage text on **stdout**, exit 2 (`S:32`); `no spec <N>` on stdout, exit 2, where the match is `specs/<N>-*.md`, a regular file (a directory fails), the **first in bytewise order** (the repo has duplicate numbers 03, 04, 05, 16, 36; do not depend on locale); `--resume` with no value → the shell's `--resume needs a session id` message on stderr and exit 1 (`S:39`, measured by the capture: golden `resume-no-value`; an earlier review's 127 was a `bash -c` artefact); any other unknown argument, including `--foo`, is the extra-brief path and the last one wins, except that `--dry-run` is accepted and ignored; the dirty check (`S:44`) comes **before** the agent check, so a dirty tree with `PII_AGENT=bogus` exits 8, not 2; then `PII_AGENT` other than `claude`/`pi` → `PII_AGENT must be claude or pi`, exit 2.
- R3 (environment). Every read treats empty as unset. `ALLOW_DIRTY` allows a dirty tree for **any non-empty value, including `0`**; `PII_LANE_GRAPH` selects the spec lane only when it equals `spec` exactly (`Spec` is the coder lane); `PII_GRAPH_EDIT_OK` applies only when it equals `1` exactly (`true` does not); an empty `PII_AGENT` means claude; the budget default is **3.00 on the coder lane, written as the raw text**, and 1.00 on the spec lane (`S:51`, `S:187`); the pi provider line is as `S:53`. All environment is read once, with `std::env::var_os`, into one `LaneEnv` built in the dispatch and passed into the core.
- R4 (the brief). Byte-identical to the heredoc for the lane (the scratch phrase differs by lane), plus the extra-brief file appended after two newlines when given. A **missing** extra-brief file still appends the two newlines, sends the error to stderr, and the run continues with exit 0. The brief is written to `brief-<tag>.md` **before** the `'''` check, so an abort on `'''` leaves the file on disk; the check applies on both lanes. The byte count printed is bytes (`wc -c`), not characters.
- R5 (coder lane). The graph is written to `_tmp/devbox-test/graphs/q<N>-<ts>.toml` with `max_wall_ms = PII_NODE_TIMEOUT_MS + 300000`, the claude or pi agent lines, the `sid = "…"` line **only for claude and only with `--resume`** (otherwise the empty line stays), and the brief with **all trailing newlines stripped** (`S:133`), while the brief file and the JSON input keep them. One timestamp `ts` feeds the log name, the graph name and the `== graph q<N>-<ts>` header.
- R6 (the checks, in the script's order). Wall and deadline first (`S:90-95`): the first `max_wall_ms = <digits>` line of the working-tree `graphs/spec.toml`, else `ABORT: graphs/spec.toml has no numeric max_wall_ms: nothing started`, exit 2. Then the resume line (R9). Then the HEAD check (`S:172`): for `spec`, `docs`, `preflight` in that order, a graph that differs from `HEAD` (staged edits and mode-only changes count; any git failure counts as "differs") or is untracked and not ignored aborts with `ABORT: graphs/<g>.toml differs from HEAD (the pin protects the reviewed graph): nothing started`, exit 2, naming the **first** such graph; `PII_GRAPH_EDIT_OK=1` skips it with its NOTE line. A graph absent from both HEAD and the tree passes the loop and is caught by the `preflight.toml is missing` abort (`S:180`). Then the pin (`S:177`): the **first** `sha256 = "[0-9a-f]*"` anywhere in `graphs/spec.toml` (comments included) must equal the SHA-256 of the working-tree `graphs/docs.toml`, else the `is not the pin` abort with both digests, exit 2 (a missing `docs.toml` gives an empty digest, printed as `''`). The wall, the pin and `max_micro_usd` are read from the **working tree**, not from HEAD.
- R7 (the input). Written to `input-<tag>.json` with the keys `agent, model, provider, spec, base, workdir, brief, docs_brief, max_budget_usd, sid` in that order. The bytes are those of Python's `json.dumps` with default separators (one line, `", "` and `": "`, `ensure_ascii`, no trailing newline). `max_budget_usd = min(PII_MAX_BUDGET_USD, [limits] max_micro_usd / 1e6)` with the limit read with the `toml` crate from the working tree. An unparseable budget **truncates the file to zero bytes**, then prints `ABORT: could not build the input` and exits 2; a missing `max_micro_usd` aborts the same way without truncating. `base` is the HEAD sha and is `""` (not an abort) when `git rev-parse HEAD` fails. `sid` is null unless claude and `--resume`.
- R8 (resume and preflight, through the trait). Claude with `--resume <sid>`: the lookup `Devbox::resume_exists(sid)`. Found → the `== dry run: session <sid> is not checked in the devbox` line is printed in dry mode only, and the sid goes into the graph or input. Not found → the `RESUME: … starting fresh` line on stdout and in the log, the sid absent from the graph and null in the input. With `PII_AGENT=pi`, `--resume` prints nothing and sets nothing. The container preflight is `Devbox::has_uv()`; its dry-mode text is `PREFLIGHT: skipped (dry run)`. The dry implementation answers "found" for any sid and "has uv", and prints exactly the script's dry-run lines.
- R9 (outputs and their order). The printed block (`AGENT`, `MODEL`, `PROVIDER`, `SPEC`, `BRIEF`, `INPUT` or `GRAPH`, `LOG`, `WAIT` with the margin's raw text) then the log file `<tag>.<ts>.log` (written only after every check passed, truncating) then `PREFLIGHT:` then `DRY RUN: nothing started, nothing spent`. On every abort path no log is written; files already written (brief, and the input when truncated) stay. A stale `input-<tag>.json` or graph from an earlier run is left alone.
- R10 (the handoff to part 2). The core returns `LanePlan { stdout: Vec<u8>, stderr: Vec<u8>, files: Vec<(PathBuf, Vec<u8>)>, exit: i32, deadline_s, log: PathBuf, graph_name, input: Option<PathBuf>, tag, sid: Option<String> }`; a thin shell writes the streams and files and exits. `pii lane plan --emit` prints the part-2 fields as `KEY=value` lines. Time is injected: the core takes `ts` and `now`.

### Non-Functional
- Rust, clippy and fmt at zero; every file under 450 effective lines; the split is a pure `plan()` core in `cmd_lane/` plus a thin IO shell, and the lint decides the file layout. `plan()` is `plan(LaneEnv, Args, &RepoSnapshot, &dyn Devbox, ts) -> LanePlan` and does no IO of its own.
- No `println!`: `writeln!(std::io::stdout(), …)` as `cmd_deploy_card.rs:127-135`. Time: one `now_secs()` with an allow-with-reason as `cmd_loop.rs:24-25`. Git: `Ctx::root(CapSet{kinds: PROC, proc_argv0: ArgvScope::new(vec!["git".into()]), ..}, …)` then `ctx.proc()?.run(&argv, root, timeout)`, as `cmd_run.rs:220-224`. No direct `Command` spawn.
- `toml = { workspace = true }` in `crates/pii-cli/Cargo.toml`; `sha2` is already there; xtask deps-check confines only redb, rustls, axum, rmcp and wasmtime.

## Architecture
```
pii lane plan N ─► LaneEnv (var_os, once) ─► plan(env, args, snapshot, &dyn Devbox, ts)
                                                  │ pure: no IO
                                                  ▼
                                       LanePlan { stdout, stderr, files, exit, … }
                                                  │
                                    thin shell: git snapshot in, files/streams/exit out
```
Decisions: (1) text scans where the shell greps (pin, wall) and the `toml` crate only where the shell uses `tomllib` (`max_micro_usd`). (2) The core is IO-free so most goldens test it in-process with a fixed `ts`. (3) It lives in pii-cli as `cmd_lane/`; move the pure core into a library crate only when an in-daemon caller (cycle-as-a-graph) exists, which is a file move, not a redesign. (4) The shell is not retired and does not yet call the port; that strangler step is its own small spec after parity is green.

## Oracle
Goldens are captured from the shell **in a scratch clone** whose `REPO=` line (`S:21`) is rewritten to the clone, never against the live tree. The procedure is committed as `crates/pii-cli/tests/golden/lane/CAPTURE.md` (clone, `sed` the `REPO=` line, run each case with the listed env, apply the substitutions). Substitutions are exact strings for known values (`<REPO>`, `<HEAD>`) and one regex only for a ten-digit `<TS>`, with the extra assertion that every `<TS>` occurrence in one run is equal. The input file is compared **as bytes** (it must equal Python's `json.dumps` output). The live oracle test runs the real shell and the port on one scratch clone per test (parallel-safe), separates stdout and stderr, requires `uv` for the spec lane (it fails, not skips, when `uv` is missing), and needs no `.git` of the repo under test beyond the scratch clone it creates; a named exclusion list covers the declared divergences.

## Declared divergences (the only ones)
1. The port's `git` runs with `ProcCap`'s scrubbed environment (7 variables, `childproc.rs:255`), so `GIT_DIR`, `GIT_INDEX_FILE`, `GIT_WORK_TREE` are not inherited.
2. `PII_NODE_TIMEOUT_MS` and `PII_WAIT_MARGIN_S`: the shell evaluates them as bash arithmetic (`abc` → 127, `1e3` → 1, leading zero → octal). The port accepts only a decimal integer and otherwise prints `ABORT: <NAME> is not a decimal integer: nothing started` and exits 2, before anything is written.
3. `PII_MAX_BUDGET_USD=nan|inf` is refused with `ABORT: could not build the input`, exit 2 (the shell writes invalid JSON).
4. `MODEL`, `PROVIDER` and the sid are escaped when interpolated into TOML; the shell does not escape.

## TDD Contract
Acceptance test: `crates/pii-cli/tests/lane_plan_parity.rs`. The requirement a plausible implementation gets wrong: **everything looks right and one artefact differs by a byte** (trailing newline of the graph's brief, the empty `sid` line, the coder budget default, JSON separators). Hence bytes, per artefact.

Each test uses its own scratch repo. Names that must exist and pass:
`spec_lane_matches_the_golden`, `coder_lane_matches_the_golden`, `resume_found_and_not_found_match_their_goldens`, `pi_ignores_resume`, `usage_is_on_stdout_exit_2`, `unknown_spec_picks_the_bytewise_first`, `three_does_not_match_thirty`, `resume_without_value_exits_127`, `dry_run_flag_is_not_a_brief_path`, `dirty_before_agent_exits_8`, `allow_dirty_zero_still_allows`, `allow_dirty_empty_does_not`, `lane_graph_must_be_exactly_spec`, `edit_ok_must_be_exactly_one`, `wall_check_precedes_head_and_pin`, `two_edited_graphs_name_the_first`, `staged_edit_is_refused`, `mode_only_change_is_refused`, `ignored_untracked_graph_passes`, `missing_preflight_graph_aborts`, `pin_is_the_first_match`, `wall_is_the_first_match`, `wrong_pin_names_both_digests`, `graph_brief_is_stripped_input_brief_is_not`, `coder_budget_default_is_three_raw`, `budget_capped_at_the_graph_limit`, `unparseable_budget_truncates_the_input`, `nan_budget_is_refused`, `missing_extra_brief_is_not_an_error`, `triple_quote_leaves_the_brief_file`, `byte_count_counts_bytes`, `one_ts_everywhere`, `no_log_on_any_abort`, `input_equals_python_json_dumps`, `input_keys_cover_every_graph_key` (derived from the graphs, porting `lane-input-matches-graph.test.sh`), `matches_the_shell_reference_in_a_scratch_clone`, and controls `a_clean_tree_passes`, `an_unchanged_graph_passes`, `a_budget_below_the_limit_is_kept`, `the_edit_opt_out_prints_its_note`.

## Exit Criteria
- [ ] `cargo test -q -p pii-cli --test lane_plan_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test lane_plan_parity -- --exact spec_lane_matches_the_golden coder_lane_matches_the_golden resume_found_and_not_found_match_their_goldens wall_check_precedes_head_and_pin pin_is_the_first_match graph_brief_is_stripped_input_brief_is_not input_equals_python_json_dumps matches_the_shell_reference_in_a_scratch_clone 2>&1 | grep -q "8 passed"`
- [ ] `git ls-files --error-unmatch crates/pii-cli/tests/golden/lane/CAPTURE.md crates/pii-cli/tests/golden/lane/spec-lane/stdout.txt`
- [ ] `git diff --quiet HEAD -- crates/pii-cli/tests/golden`
- [ ] `cargo build -q -p pii-cli --bin pii`
- [ ] `cargo clippy -q -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `cargo run -q -p xtask -- net-check`
- [ ] `git diff --quiet HEAD -- graphs scripts/run-batchC-pii.sh`

- [ ] `test $(find crates/pii-cli/src/cmd_lane -name '*.rs' -size +0 | wc -l) -ge 6`
## Files
The queue refuses directory paths, so the layout is fixed here. Every file below must exist and be non-empty when the row ends, and none may exceed 450 effective lines.
- `crates/pii-cli/src/main.rs`
- `crates/pii-cli/src/cmd_lane/mod.rs`
- `crates/pii-cli/src/cmd_lane/args.rs`
- `crates/pii-cli/src/cmd_lane/env.rs`
- `crates/pii-cli/src/cmd_lane/brief.rs`
- `crates/pii-cli/src/cmd_lane/checks.rs`
- `crates/pii-cli/src/cmd_lane/input.rs`
- `crates/pii-cli/src/cmd_lane/git.rs`
- `crates/pii-cli/Cargo.toml`
- `Cargo.lock`
- `crates/pii-cli/tests/lane_plan_parity.rs`

## Guardrails
- Do NOT edit `scripts/`, `graphs/`, `specs/`, the golden files, or any existing test assertion. The script is the reference and a golden is the contract. If they disagree, report it; do not fix either.
- Do NOT normalise anything the substitutions do not name. A difference you cannot explain is the finding.
- Appearance-only implementations to refuse: a port that prints the right lines but writes different files; a `contains` check on any artefact; a live oracle that touches the real `_tmp/claude-batchC`.
- Mutations the tests must kill (each killed by a named test above): log and graph with different timestamps; `base` or the root taken from the cwd; the graph's brief unstripped, or the input's stripped; a missing extra-brief file treated as an error; `pi` + `--resume` printing the line or setting the sid; `ALLOW_DIRTY == "1"` instead of non-empty; `Spec` matched case-insensitively; `PII_GRAPH_EDIT_OK` as a truthy parse; the index compared instead of `HEAD`; the pin from the last match; the wall from a TOML parser; the dirty check after the agent check; the wall check after the HEAD check; a log written before the checks; characters counted instead of bytes; a spec match without the dash; a locale sort.
