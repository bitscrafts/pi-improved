# pii-cli Test Gaps and Small Hygiene Batch

- **Project**: `/projects/pi-improved`
- **Crate**: `pii-cli`, `pii-http-fetch`, `pii-channels` (doc comment
  only), `pii-web` (doc comment only), root `Cargo.toml` (comment
  only)
- **Priority**: LOW-MEDIUM — the `pii-cli` piece closes real
  cheap-to-test gaps in pure logic; the rest is small hygiene, bundled
  because each item is a one-to-few-line change not worth its own
  spec. Sourced from fable review candidates 9 and 10,
  `_docs/06-research/032-fable-pii-review-2026-09-18.md`.

## Overview

Two parts.

### Part A: pii-cli pure-logic test gaps (candidate 9)

`crates/pii-cli` has no `tests/` directory; its 19 existing tests are
inline in `main.rs`, `ndjson.rs`, `cmd_trace.rs`, `cmd_oneshot.rs`.
Several command files have pure, cheap-to-test logic with zero
coverage today (confirmed via `grep` for `#[cfg(test)]`/`#[test]` —
all return no matches in these files):

- `cmd_run.rs`: `Common::parse` (argv parsing), `root_ctx` (CapSet/
  Budget construction, path canonicalization), `report()` (maps
  `Result<Terminal, PiiError>` to exit codes 0/1/4).
- `cmd_sys.rs`: `run_ls`'s `--graph` filter parsing (client-side, pure,
  no daemon needed).
- `cmd_sandbox_exec.rs`: the exit-code coercion `s.code().unwrap_or(1)
  as u32 & 0xff` (line ~65).
- `cmd_daemon.rs`: exit-code selection logic (the specific codes 2/4/5/
  130 the daemon bootstrap path returns — search for where these
  literals appear) and `self_uid()`'s `/proc` parsing.
- `cmd_soak.rs`: status matching against the literal strings
  `"succeeded"`/`"failed"` from a `RunGet` response — brittle string
  matching where a rename on the daemon side would silently break
  this file with no test to catch it.

`cmd_worker_exec.rs` needs NO new test — it is already exercised as a
real subprocess by `pii-worker/tests/supervisor.rs`'s `worker_spec()`
helper (verify this is still true before skipping it).
`cmd_ctl.rs`/`cmd_tui.rs` are lower-value / filesystem-coupled per the
review; not required by this spec, may be added if convenient but
don't let them block landing the higher-value pieces.

### Part B: small hygiene, one-to-few-line fixes each

1. `crates/pii-http-fetch/src/lib.rs` (~line 96): the fetch audit log
   line hardcodes the literal text `200-OK` regardless of the actual
   HTTP status — a 404 response is logged as `200-OK status=404`. The
   real `status` variable IS present in the format string already;
   only the literal `200-OK` text is wrong. Fix: remove the hardcoded
   `200-OK` text (the `status={status}` field already carries the real
   information); do not invent new status-classification logic.
2. `crates/pii-cli/src/cmd_daemon.rs` (~line 249): `let _ =
   std::fs::set_permissions(&sock_path, ...)` silently discards a
   chmod failure. The surrounding comment describes this chmod as part
   of the mode-discipline defense the file's `disallowed_methods`
   allow relies on. Fix: on failure, print a message to stderr (follow
   this file's existing error-reporting convention — check how other
   non-fatal failures in the same function are surfaced) and continue
   (do NOT make this fatal — the socket directory's own 0700
   permissions already provide defense in depth per the existing
   comment; this is a logging gap, not a security hole).
3. Root `Cargo.toml`: `ratatui = "=0.30.2"` is an exact-version pin
   with no comment explaining why (unlike the `.cargo/config.toml`
   `CC = clang` override, which does have a reason comment). Add a
   one-line comment explaining the pin's reason if you can determine
   it (check git blame/history for the commit that introduced the
   pin), or relax it to `"0.30"` if no reason is discoverable and nothing
   in the workspace depends on the exact patch version. State which
   choice you made and why in your report.
4. `crates/pii-channels/src/discord.rs` (module doc comment, top of
   file): says "TLS is cut to Phase 3b, so `PII_DISCORD_API_BASE` must
   point at plaintext `http://`" — but `pii-net` now supports TLS (a
   later phase already landed it), and `DEFAULT_API_BASE` in this same
   file is already `https://`. The comment is stale. Fix: update it to
   reflect that TLS is available now (verify by checking whether
   `pii-net`'s TLS support is actually usable by this channel today,
   not just present in the crate — read how `DiscordChannel` obtains
   its `HttpTransport` before writing the new comment, so it states
   the ACTUAL current capability, not an assumption).
5. `crates/pii-web/src/main.rs` (crate-level or module-level doc
   comment): the crate's `--host 0.0.0.0` flag is accepted (there's an
   existing test for it) but the crate has no authentication of its
   own — it forwards the daemon's full read surface under `pii-web`'s
   own process uid, bypassing the peer-uid check the daemon's UDS
   socket normally relies on. This is a documented design choice per
   the review (not a bug to fix in this spec), but it isn't documented
   IN the crate today. Add a doc comment noting that binding off
   loopback exposes the daemon's read surface without additional
   authentication, so operators understand the tradeoff before
   choosing `--host 0.0.0.0`.

Ground truth (verify yourself, re-read every file — line numbers will
have moved since this spec was drafted): every file path and line
number above was checked directly before writing this spec, but this
project's convention is that specs may go stale between writing and
implementation — always re-read before editing.

## Requirements

### Functional

- R1 (Part A). Add tests for `Common::parse`, `root_ctx`, and
  `report()` in `cmd_run.rs`; for `run_ls`'s `--graph` filter in
  `cmd_sys.rs`; for the exit-code coercion in `cmd_sandbox_exec.rs`;
  for exit-code selection and `self_uid()` in `cmd_daemon.rs`; for the
  status-string matching in `cmd_soak.rs`. Use inline `#[cfg(test)]
  mod tests` in each file (matching this crate's existing convention —
  no `tests/` directory exists and this spec does not introduce one).
- R2 (Part B, item 1). Fix the `200-OK` literal in
  `pii-http-fetch/src/lib.rs` as described above.
- R3 (Part B, item 2). Log the chmod failure in `cmd_daemon.rs` as
  described above.
- R4 (Part B, item 3). Add a reason comment or relax the `ratatui`
  pin in root `Cargo.toml`, per the investigation described above.
- R5 (Part B, item 4). Update the stale TLS comment in
  `pii-channels/src/discord.rs`.
- R6 (Part B, item 5). Add the loopback-exposure doc note to
  `pii-web/src/main.rs`.

### Non-Functional

- N1. Part A tests are pure unit tests — no daemon, no real subprocess
  spawn, no network. If any of the targeted functions turn out to
  require more than that to test meaningfully, report which one and
  why rather than building test infrastructure beyond this spec's
  scope.
- N2. Part B items are each a small, self-contained, low-risk change —
  do not let any one of them turn into a larger refactor. If
  implementing one reveals it's more involved than described (e.g.
  the `ratatui` pin turns out to be load-bearing for a reason not
  found in git history), STOP and report rather than guessing.

## Architecture

Described inline in Overview above.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `common_parse_handles_valid_args` | representative argv for `cmd_run`'s `Common::parse` | parses into expected fields |
| `root_ctx_builds_expected_capset` | representative inputs | `Ctx` with expected `CapSet`/`Budget` |
| `report_maps_terminal_states_to_exit_codes` | each `Terminal` variant / `PiiError` case `report()` handles | correct exit code (0/1/4) per case |
| `run_ls_parses_graph_filter` | argv with `--graph foo` | filter value `"foo"` extracted correctly |
| `sandbox_exec_exit_code_coercion` | various `ExitStatus` values including `None` (killed by signal) | correct `u32 & 0xff` result, `1` fallback for `None` |
| `cmd_daemon_exit_code_selection` | each bootstrap failure case | matches the documented 2/4/5/130 codes |
| `self_uid_parses_proc_status` | a representative `/proc/self/status`-shaped fixture | correct uid extracted |
| `cmd_soak_status_matching` | `"succeeded"` / `"failed"` / an unexpected string | correct handling of all three, including the unexpected case |

## Exit Criteria

- [ ] `cargo test -p pii-cli`
- [ ] `! grep -q '200-OK' crates/pii-http-fetch/src/lib.rs`
- [ ] `cargo build -p pii-cli -p pii-http-fetch -p pii-channels -p pii-web`
- [ ] `cargo clippy -p pii-cli -p pii-http-fetch -p pii-channels -p pii-web -- -D warnings`
- [ ] `cargo fmt --check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- Do NOT add a `tests/` directory to `pii-cli` — use inline
  `#[cfg(test)]` modules matching the existing convention.
- Do NOT make the chmod failure in `cmd_daemon.rs` fatal — logging
  only, per R3.
- Do NOT relax the `ratatui` pin without first checking git history
  for a reason (R4) — report which you found.
- Do NOT modify any file under `specs/`.
- No emojis, no AI-tool mentions in any comment or commit message.
- If any Part A function is not as pure/testable as described (e.g.
  it has hidden I/O or global state that makes unit testing
  impractical without larger refactoring), STOP and report that
  specific function rather than forcing a test around it.
