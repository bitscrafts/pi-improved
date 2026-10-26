# Spec 281: a spend-limit phrase the agent READ is not the CLI's refusal

## Overview
**Project**: /projects/pi-improved
**Phase**: production — unattended queue. Supersedes nothing; narrows spec 237's detector.

Between 00:02 and 00:11 UTC on 2026-10-26 the queue ran eight cycles on one row (A145) in nine minutes. Each worker finished cleanly — `rc=0`, `claude_is_error=false`, `end_turn`, 14–50 s, 0.010–0.018 USD — and each was journalled `quota_exhausted=yes` with `quota_reset=resets 8:20pm (UTC)`. The reset time was not the account's: it was read out of `scripts/autonomy/selftest.sh:51`, a test fixture the worker had opened:

```
fake x 'echo "You have hit your session limit - resets 8:20pm (UTC)" >&2; exit 1'
```

`crates/pii-server/src/node_kinds/nodes/agent_quota.rs` looks for the CLI's phrase with `lower.contains(marker)` over the WHOLE captured stdout, and a claude stream echoes everything the agent read in its `tool_result` lines. So a worker that opens a file quoting the phrase is indistinguishable from one the account refused. The visible cost was eight wasted cycles (~0.12 USD) and a loop that could not stop; the deeper cost is that the signal is worthless on any repository that mentions the phrase — which this one does in four files (`selftest.sh`, `window.sh`, `probe.sh`, and the specs that quote them).

## Verified Premises
- **CONFIRMED** (read 2026-10-26) `quota_signal(output, stderr)` = `is_limit()` over each of the two texts; `is_limit` lowercases and `contains` any of five markers; `report.rs::quota_fields` journals the result on `coder.exited`. Nothing else reads it today (the doc comment says observability only).
- **CONFIRMED** (measured) the eight `coder.exited` events of run `r000000000225` and its eight siblings carry `quota_exhausted=yes` with `claude_is_error=false`, `stop_reason=end_turn`, `rc=0`, and the reset text `resets 8:20pm (UTC)`, which is `selftest.sh`'s fixture, not a CLI message.
- **CONFIRMED** the phrase occurs in `scripts/autonomy/selftest.sh:51`, `scripts/autonomy/window.sh:18`, `scripts/autonomy/probe.sh:35`, `specs/237-…` and `specs/280-…`.
- **NOT CLAIMED**: that spec 237's original behaviour was wrong for plain-text output — it was not, and R2 keeps it.
- **NOT CLAIMED**: that this is why A142's 18:27 run stopped. That run's daemon predates spec 263's head+tail capture, so its `claude_is_error` was `unknown` and the refusal cannot be told from an echo after the fact. The A145 evidence above is sufficient on its own.

## Requirements
### Functional
- **R1. On a JSON stream, only structure counts.** When stdout is a JSON stream (any line parses as a JSON object), a quota signal comes only from a line that carries it as structure: `api_error_status == 429`; or a `type` of `rate_limit_error`, at the top level or under `error.type`; or a line whose `type` is `result` with `is_error` true AND whose own `result` text holds one of the five markers. A marker inside any other line — `tool_result`, `assistant`, a system line — is not a signal. The reset text is read from the line that carried the signal.
- **R2. Plain text is unchanged.** Output that is not a JSON stream is scanned whole, exactly as spec 237 does, so a CLI that prints its refusal as text still reports. stderr is the CLI's own stream and is always scanned whole.
- **R3. Tests, with controls.** `crates/pii-server/tests/coder_quota_signal.rs` keeps its ten existing tests unmodified and gains six: (a) THE BUG — a stream whose `tool_result` quotes the phrase and whose `result` line is `is_error: false` reports nothing; (b) CONTROL — the same stream with a real `result`/`is_error: true`/429 refusal line reports, with the reset text from that line; (c) the refusal is found with clean lines around it; (d) NEAR MISS — a `result` line that is not an error but quotes the phrase reports nothing; (e) a nested `error.type: rate_limit_error` reports; (f) CONTROL — plain-text output is still scanned whole, positively and negatively; and (g) the `coder.exited` event says `quota_exhausted=no` for a run that only read the phrase.

### Non-Functional
- Every file under the 450-line gate. No new dependency. `agent_quota.rs` stays small (117 lines after this). Only the detector and its test change; `report.rs`, `window.sh`, `probe.sh`, `selftest.sh` are untouched.

## Why the orchestrator wrote this instead of a worker
Every other spec in this queue is implemented by a worker through the daemon. This one cannot be: the worker must open `agent_quota.rs`, whose `MARKERS` array holds the literal `hit your session limit`, and again `selftest.sh`, which holds the reset-time fixture. The detector being fixed would fire on the worker's own reading of it and park the row as `window-closed` — the same deadlock the bug creates. Recorded as a documented exception, with the same evidence a worker's spec would demand: control red on the old detector, 17/17 green on the fix, three mutations.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_quota_signal 2>&1 | grep -qE "^test result: ok\. 17 passed"`
- [ ] `grep -q "fn output_refusal" crates/pii-server/src/node_kinds/nodes/agent_quota.rs`
- [ ] `grep -q "is_limit(result)" crates/pii-server/src/node_kinds/nodes/agent_quota.rs`
- [ ] `cargo test -q -p pii-server --test coder_charges_run_budget 2>&1 | grep -qE "^test result: ok\. [0-9]+ passed"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT touch `report.rs`, `window.sh`, `probe.sh`, `selftest.sh` or any spec's quoted text. Do NOT widen the markers. Do NOT make the detector read the agent's stderr differently.

## Follow-ups (not this spec)
1. `window.sh` and `probe.sh` read the WORKER LOG, which contains the same echoed text, so they can be fooled the same way; they should read the journal's `quota_exhausted` field instead of grepping a log.
2. A fixture that quotes a refusal belongs in a file the workers do not read; long term the phrase should be assembled at run time in the fixtures, not spelled out.

## Closure
**Committed** as `e176a5c`; the six Exit Criteria were then run by the orchestrator on that commit instead of being read from the implementing lane's log: criteria 1, 3, 4, 5, 6 returned rc=0 and criterion 2 returned **rc=1**.

**Criterion 2 amended, reason measured.** The first attempt's gate failed on this crate's own `clippy -D warnings` (`option_if_let_else`, `option_option`) and nothing was committed; the version that passes the gate folds both rules into one helper, `output_refusal`, which takes the structured line when stdout is a JSON stream and falls back to the whole-text scan when it is plain output. The criterion named the pre-refactor helper, so it could not pass on the code the gate accepts. It now names the function that exists — the same assertion (a dedicated helper decides the stream case), nothing else relaxed. Criteria 1, 3 and 5 were green before the edit and after it, so the amendment bought no other pass.

**The defect is reddened, and the file restored byte-identical.** Deleting `refusal_text`'s `&& is_error` condition — back to spec 237's behaviour — turned `coder_quota_signal` red on exactly one test, `an_error_free_result_that_quotes_the_phrase_is_not_a_signal` (16 passed, 1 failed). Restored with `cp` (not `cp -p`) and `touch`; `sha256` `5acc29226f5fad367d52ecf80b183467bdb82d6c4fd1ea0e7a5844ea1a0bc4f9` before and after, and the same test then read `ok. 17 passed`.
