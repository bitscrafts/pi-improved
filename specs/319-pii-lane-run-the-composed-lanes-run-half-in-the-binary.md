# Spec 319: `pii lane run` — the composed lane's run half (`run-batchC-pii.sh` lines 219-343) in the binary, so the shell lane can be retired

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the product is one binary and ships no companion scripts (user, 2026-10-29/30)

Part 1 (spec 317, landed `a9487a2`) ported everything before the run: `pii lane plan` returns a `LanePlan` and takes the two devbox probes (`resume_exists`, `has_uv`) through the `Devbox` trait, with a dry implementation. This part ports **everything after the plan**: the daemon-up call, the judge preflight, the start, the session handle, the wait, the evidence, the quota text, the spec guard and the exit code. It adds the real `Devbox`/transport implementation (ssh + `podman exec`), so `pii lane run <N> [--resume <sid>] [<extra-brief-file>]` replaces `bash scripts/run-batchC-pii.sh <N> ...` in full. The script is **not edited or deleted by this spec**: it is the reference. A later one-line row retires it once this one is proved.

`S:<n>` means line n of `scripts/run-batchC-pii.sh` at the commit that installs this spec.

## Requirements
### Functional
- R1 (transport). One `Transport` trait: `exec(&self, argv_after_host: &str) -> Output` where the real implementation runs `[$PII_SSH or ssh] -o BatchMode=yes -o ConnectTimeout=15 $DEVBOX_HOST <one string>` (`S:26-27`, host default `mvcorrea@10.10.10.217`, `DEVBOX_HOST` overrides), and a `devbox_x()` helper builds `podman exec -w <agent_root> pii-devbox <args>` (`S:229`). The real `Devbox` impl for part 1's trait is built on it: `resume_exists` runs the `find <CFG> -name "<sid>.jsonl" -type f -print -quit` probe of `S:103` and `has_uv` runs `command -v uv >/dev/null` inside the container (`S:68`, `S:211-216`) with the `ABORT: container preflight failed: <output>` text and exit 2. Output of `ssh` that is not a result goes to stderr as in the script.
- R2 (the run, in order, with the script's exact stdout lines, log appends and exit codes): daemon up through `PII_DAEMON_SH` (default `modules/devbox/daemon.sh up`; porting that file is the devbox-deploy item, not this one) → `DAEMON   : <last line>` and `ABORT: the devbox daemon is not up:` + last 5 lines, exit 2 (`S:222-224`); on the spec lane copy the three graphs into `_tmp/devbox-test/graphs/` (`S:225`); record `git status --porcelain -- specs | sort` before (`S:227`); the **judge preflight** (`S:234-255`: a throwaway repo under `_tmp/devbox-test/preflight-<ts>-XXXXXX`, a one-criterion spec, a base commit made with the neutral identity `pii-preflight`, a `run start preflight` through the daemon, polling up to 60 times with `PII_POLL_SECONDS` default 5, the four terminal words, and the two `ABORT` texts); `LINES_BEFORE` = the daemon log's line count; the start (spec lane: a generated `start-<tag>.sh` written under the log dir and run in the container; coder lane: `run start <graph name>`); the run id is the first `r` + 12 digits, else `ABORT: no run id: <first 300 bytes>`, exit 2; `RUN      : <id>` and `== run <id>` in the log.
- R3 (the session handle, `S:268-274`): up to 60 tries, 2 s apart, of `tail -n +<LINES_BEFORE+1> <C>/pii.log | grep -a coder.spawned | tail -1`, the `"session":"…"` value written to `session-<tag>.txt` and `== session handle captured at spawn: <sid>` in the log.
- R4 (the wait). Spec lane: until a terminal word (`succeeded|failed|refused|cancelled`) or `SECONDS >= DEADLINE` (part 1's `deadline_s`), then, if still running, `run cancel <id> --kill --reason lane-wait-expired` and the `== <id> still running after <n>s: cancelled by the lane` log lines. Coder lane: at most `PII_NODE_TIMEOUT_MS / 5000 + 120` polls. `== <id> final: <status|unknown> after ~<n>s`.
- R5 (the evidence). `run explain <id>` first 60 lines appended to the log; on a spec-lane run that did not succeed, the `crit=`/`fail=` lines of `run trace` under `== why the run was not accepted:` (or the `(the trace names no criterion)` form); `== coder.exited: <first line, or last line on the coder lane>`; the **facts** (`rc`, `tree_changed`, `quota_exhausted`, `claude_cost_usd`) extracted with the script's exact `grep -o` shapes — a `"key":"value"` or `"key":value` pair, first match, the quote stripped — and the `FACTS    :` and (spec lane) `DOCS-FACTS:` lines; a refusal by either coder prints `You've hit your session limit` with ` · resets <h:mm>(am|pm) (UTC)` only when the reset text has one, then `QUOTA    : refused by the <host|docs> coder`.
- R6 (the spec guard and the exit code, `S:337-343`): `comm -13 before after` of the sorted `specs` status; any new line → `SPEC-GUARD:` + the lines and **exit 9**; else `== tree after: <n> dirty paths` + the first 12 lines of `git status --porcelain`; exit 0 only when the final status contains `succeeded`, else 1.
- R7 (`--dry-run` is still part 1's). `pii lane run --dry-run` is `pii lane plan`.
- R8 (no hardcoded host, path or user): host, port-less ssh, the container name (`pii-devbox`, overridable by `PII_DEVBOX_CONTAINER`), the config dir and the agent root come from the part-1 `LaneEnv`/`LanePlan`.

### Non-Functional
- Same rules as part 1: Rust, clippy and fmt zero, every file under 450 effective lines, no `println!`, no direct `Command` spawn (`Ctx`/`ProcCap` for `ssh`, `git`), `toml` and `sha2` only, env read once into `LaneEnv`. Time and sleeping are injected (`Clock` trait) so tests run in milliseconds.
- The poll loop and the wait are pure functions of an injected `Clock` and `Transport`; no `std::thread::sleep` outside the one real `Clock`.

## Architecture
```
pii lane run N ─► plan() (part 1) ─► LanePlan
                    └─► run(plan, &dyn Transport, &dyn Clock) ─► RunReport { stdout, log_appends, files, exit }
                                                              thin IO shell writes them
Transport = ssh (real) | recorded stub (tests)
```
Decisions: (1) **Differential oracle.** The shell script already runs under a stub `ssh` (`PII_SSH`) in `scripts/autonomy/tests/lane-*.test.sh`. The new test runs **both** the shell and `pii lane run` against the same stub, which records every ssh argument string and replies from a script of canned outputs, and compares: the recorded argv sequence, stdout, stderr, the log file, `session-<tag>.txt`, the exit code. (2) The part-1 `Devbox` trait gets a real implementation, nothing in part 1's core changes. (3) `daemon.sh` stays a shell call until the devbox-deploy item; it is named, not hidden.

## Oracle and declared divergences
The recorded-argv comparison is the heart: if the port skips a probe, reorders two calls, or changes a quoting, the recorded sequence differs. The stub's canned replies cover: preflight pass/fail, no run id, success, failure, refusal by the host coder, refusal by the docs coder, wait expiry (cancelled), a spec edit by the agent (exit 9), a claude resume found/not found. Declared divergences (the only ones): `Clock` replaces real sleeping; the throwaway directory name's random suffix and every timestamp are normalised as in part 1 (`<TS>`, `<RAND>`); `ssh` runs through `ProcCap`'s scrubbed environment (7 variables).

## TDD Contract
Acceptance test: `crates/pii-cli/tests/lane_run_parity.rs`. The requirement a plausible implementation gets wrong: **everything printed is right and one remote command differs** (a quoting, a flag, an order), invisible to a stdout check but visible in the recorded ssh argv. Each test uses its own scratch repo and stub.
Tests that must exist and pass: `daemon_down_aborts_before_anything`, `preflight_failure_prints_the_judges_trace_lines`, `no_run_id_aborts_with_300_bytes`, `session_handle_is_captured_and_written`, `success_exits_0_with_facts`, `failure_exits_1_with_the_why_lines`, `host_quota_refusal_prints_the_reset_time`, `docs_quota_refusal_names_the_docs_coder`, `refusal_without_a_reset_time_prints_the_bare_line`, `wait_expiry_cancels_with_kill_and_the_reason`, `coder_lane_polls_are_bounded_by_the_node_timeout`, `agent_edit_under_specs_exits_9_and_lists_it`, `tree_after_lists_at_most_12_paths`, `facts_quote_stripping_matches_the_shell`, `the_recorded_ssh_sequence_equals_the_shells` (every scenario above, both implementations), `resume_found_and_not_found_via_the_real_devbox_impl`, `has_uv_failure_aborts_with_the_probe_output`, `ssh_binary_and_host_come_from_the_environment`, `no_hardcoded_host_in_the_source` (greps `cmd_lane` for the default host literal outside the one default constant), and the controls `a_successful_run_has_no_quota_lines` and `the_spec_guard_passes_when_specs_are_untouched`.

## Exit Criteria
- [ ] `cargo test -q -p pii-cli --test lane_run_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo test -q -p pii-cli --test lane_run_parity -- --exact the_recorded_ssh_sequence_equals_the_shells facts_quote_stripping_matches_the_shell wait_expiry_cancels_with_kill_and_the_reason agent_edit_under_specs_exits_9_and_lists_it 2>&1 | grep -q "4 passed"`
- [ ] `cargo test -q -p pii-cli --test lane_plan_parity 2>&1 | grep -q "test result: ok"`
- [ ] `cargo build -q -p pii-cli --bin pii`
- [ ] `cargo clippy -q -p pii-cli --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`
- [ ] `cargo run -q -p xtask -- net-check`
- [ ] `test $(find crates/pii-cli/src/cmd_lane -name '*.rs' -size +0 | wc -l) -ge 12`
- [ ] `git diff --quiet HEAD -- graphs scripts crates/pii-cli/tests/golden`

## Files
- `crates/pii-cli/src/main.rs`
- `crates/pii-cli/src/cmd_lane/mod.rs`
- `crates/pii-cli/src/cmd_lane/transport.rs`
- `crates/pii-cli/src/cmd_lane/devbox.rs`
- `crates/pii-cli/src/cmd_lane/clock.rs`
- `crates/pii-cli/src/cmd_lane/run.rs`
- `crates/pii-cli/src/cmd_lane/preflight.rs`
- `crates/pii-cli/src/cmd_lane/wait.rs`
- `crates/pii-cli/src/cmd_lane/facts.rs`
- `crates/pii-cli/src/cmd_lane/guard.rs`
- `crates/pii-cli/tests/lane_run_parity.rs`

## Guardrails
- Do NOT edit `scripts/`, `graphs/`, `specs/`, the part-1 goldens, or any existing test assertion. The shell lane is the reference and stays runnable.
- Do NOT port `modules/devbox/daemon.sh`: call it, name it.
- Appearance-only implementations to refuse: matching stdout while changing a remote command; a stub that answers whatever it is asked (it must fail on an unexpected argument); a `Clock` that never advances.
- Mutations the tests must kill: the preflight skipped on the spec lane; `--kill` dropped from the wait-expiry cancel; the reason text changed; `LINES_BEFORE` taken after the start; the session grep matching `coder.exited`; the facts taken from the last line instead of the first on the spec lane; the docs refusal ignored; the reset time printed when absent; the `specs` snapshot taken after the start; exit 9 swallowed; `tree after` unbounded; the poll count not bounded by the node timeout.
