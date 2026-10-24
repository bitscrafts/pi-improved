# Spec 253: the journal carries the text claude gave for its own failure

## Overview
**Project**: /projects/pi-improved
**Phase**: production — a failed agent must say why, in the journal (user directive 2026-10-24: all specs are implemented by pii)

The first real queue item through pii died in 5 s: claude called the run an error with zero tokens and zero cost, and `run explain` said "agent failed, no
stderr". The cause (HTTP 401, "OAuth token revoked") was in claude's final `result` event, which carries `is_error`, `api_error_status` and a `result`
string. `agent_claude::run_facts` already reads that event for `is_error`, turns and cost, but not for those two fields, and the journal's
`agent_output_head` keeps only the first 200 characters, which here is the `init` event. The cause was found only by calling claude by hand.

## Requirements
### Functional
- **R1.** `RunFacts` (crates/pii-server/src/node_kinds/nodes/agent_claude.rs) gains `result_text: Option<String>` (the `result` string of the `result` event)
  and `api_error_status: Option<u64>` (its `api_error_status`, when a number). `None` when absent: unknown, not empty.
- **R2.** The coder node's `coder.exited` event gains two fields written next to `claude_is_error` in coder/report.rs: `claude_result` (the result text,
  cut to at most 300 characters; empty string when `None`) and `claude_api_error_status` (the number as a string; empty when `None`).
- **R3.** The `claude_result` value goes through the SAME redaction the other output fields use (coder/redact.rs): a planted secret in the result text must
  not appear in the journal.
- **R4.** A run that is not a claude run leaves both fields empty; nothing else in `coder.exited` changes.

### Non-Functional
- Observability only: no verdict, route or exit code changes. Files stay under the 450-line gate (report.rs is 190 lines; agent_claude.rs must stay under 450).

## Architecture
```
claude stdout (stream-json) --run_facts--> RunFacts{result_text, api_error_status} --report.rs--> coder.exited{claude_result, claude_api_error_status}
                                                                                       ^ redact.rs on the text
```

## TDD Contract
New file `crates/pii-server/tests/coder_journals_the_claude_result.rs`, at least these tests (names are the contract):

| Test name | Input | Expected |
|---|---|---|
| `run_facts_reads_the_result_text` | a `result` line with `"result":"Failed to authenticate"` | `result_text == Some("Failed to authenticate")` |
| `run_facts_reads_the_api_error_status` | a `result` line with `"api_error_status":401` | `api_error_status == Some(401)` |
| `absent_fields_are_none` | a `result` line with neither | both `None` (not `Some("")`, not `Some(0)`) |
| `the_journal_carries_the_result_and_status` | a coder run whose fake agent prints a failing `result` event and exits 1 | `coder.exited` has `claude_result` = that text and `claude_api_error_status` = `401` |
| `a_long_result_is_cut_at_300_characters` | a 1000-character result | the journalled value is exactly 300 characters |
| `a_secret_in_the_result_is_redacted` | a result containing a planted `sk-ant-` shaped string | the journalled value does not contain it |
| `a_non_claude_run_leaves_both_fields_empty` | a pi-style fake agent | both fields are the empty string |

Reuse the fake-agent pattern of `crates/pii-server/tests/agent_claude_session_handle.rs` for the journal tests.

## Exit Criteria
- [ ] `cargo test -q -p pii-server --test coder_journals_the_claude_result 2>&1 | grep -qE "^test result: ok\. [7-9] passed|^test result: ok\. [1-9][0-9] passed"`
- [ ] `cargo test -q -p pii-server --test agent_claude_session_handle 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --test agent_claude_arm 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo test -q -p pii-server --lib 2>&1 | grep -qE "^test result: ok"`
- [ ] `cargo clippy -q -p pii-server --all-targets -- -D warnings`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT change a verdict, a route, an exit code or the existing field names of `coder.exited`.
- Do NOT print the raw `result` event or any credential anywhere outside the redacted journal field.
- Do NOT edit any existing test to make it pass; a failing existing test is a finding to report.
- Do NOT commit.
