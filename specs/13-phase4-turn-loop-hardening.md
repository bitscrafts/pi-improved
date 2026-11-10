# Spec: Phase 4.1a — Turn-loop hardening: provider options, env scrubbing, process-group, grep/glob tools

**Project**: <repo>
**Phase**: 4 — "daily driver" (opus review 2026-09-07, `_docs/06-research/007-pii-phase4-plan.md`)
**Status**: DRAFT — spec only, no implementation authorized yet. User approved
Phase 4 ("so proceed to the phase 4 with pidag!") — this is the first P4.1
slice. Implementation follows via `pidag sdd` with a `deepseek/deepseek-v4-flash`
worker (thinking OFF mandatory).

## Overview

Opus's Phase-4 verdict: *"production-grade substrate carrying an MVP-grade
product."* The agent turn loop (`crates/pii-server/src/agent.rs`) is the
thinnest, most user-facing layer, and it is full of **silent no-ops** — features
shaped but not wired. This spec fixes the four cheapest, most self-contained
ones, all in the tool/provider/child-process layer. Streaming and compaction
are deliberately deferred to a follow-up spec (4.1b) because streaming needs a
streaming transport, which is a larger architectural change than the sync
`HttpTransport` seam allows today.

The four defects, verified against the current tree:

1. **Provider options are built then dropped.** `ChatRequest` carries
   `max_tokens` and `temperature` (`crates/pii-model/src/lib.rs:90-93`), and
   `AgentNode::run` sets them (`agent.rs:119-120`), but `OllamaProvider::build_request`
   (`crates/pii-model/src/ollama.rs:80-87`) sends only `model`, `messages`,
   `tools`, and a hardcoded `"stream": false`. `temperature: 0.0` and
   `max_tokens: 4096` are lies — the model never sees them. `num_ctx` is not
   sent at all.

2. **The model's `bash` inherits the daemon's full environment.** `run_child`
   (`crates/pii-core/src/ctx.rs:538-601`) builds `Command::new(argv0)` with no
   `env_clear`/`envs`, so every child inherits every API key the daemon holds
   (`NVIDIA_API_KEY`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, …). Landlock is
   filesystem-only, so the model's shell can `curl` anywhere and read any
   secret in the daemon's env. This is the hard prerequisite for P4.4 (TLS).

3. **A timed-out `bash` orphans its grandchildren.** `run_child` kills only the
   direct child on timeout (`ctx.rs:591`), not the process group. A timed-out
   `cargo test` leaves the `cargo` process's children running.

4. **The tool set is too thin.** Only five tools (`fs_read`, `fs_write`,
   `edit`, `multi_edit`, `bash` — `crates/pii-tools/src/lib.rs:45-63`). A small
   local model burns turns doing `bash: find . -name` instead of a `glob`/`grep`
   call. The plan calls these "cheap; the difference between a 3B model
   succeeding and thrashing."

## Requirements

### Functional

#### F1 — Provider options actually reach the wire
`OllamaProvider::build_request` must send `temperature`, `num_predict`
(derived from `req.max_tokens`), and `num_ctx` (from a new `num_ctx` field on
`ChatRequest`, defaulting to a sane value) in the Ollama `/api/chat` body.
`stream` stays `false` (streaming is 4.1b). The `ChatRequest` struct gains a
`num_ctx: u32` field; `AgentNode::run` sets it from a new `num_ctx` agent-node
config key (default 8192). `temperature` and `max_tokens` must be forwarded
verbatim from the request.

#### F2 — Child processes get a scrubbed environment
`run_child` (and `spawn_child`) must NOT inherit the daemon's full environment.
Instead, build the child env from an explicit allowlist: `PATH`, `HOME`, `LANG`,
`LC_ALL`, `TERM`, `PWD`, `TMPDIR`, plus any `PII_*` variables (so `PII_BASH_TIMEOUT_MS`
and the like still work). All other variables — including every `*_API_KEY` —
are dropped. The allowlist is a `const` in `pii-core`; `ProcCap::run` and
`ProcCap::spawn_child` pass it through. This is defense-in-depth on top of
Landlock (which is filesystem-only and cannot stop `curl`).

#### F3 — Timeout/cancel kills the whole process group
`run_child` must spawn the child in its own process group (`Command::process_group(0)`,
stable, no `unsafe`) and, on timeout, kill the group (`killpg` via the child's
pid) rather than only the direct child. The `try_wait` loop already polls every
10ms; add a `ctx.check()`-driven cancellation observation so a `run.cancel`
during a long `bash` is honored (the loop returns `PiiError::Cancelled` when the
context is cancelled). `ProcOutput` gains a `cancelled: bool` field (default
false) so callers can distinguish a cancel from a timeout.

#### F4 — Two new tools: `grep` and `glob`
Add `GrepTool` and `GlobTool` to `pii-tools`, registered in `ToolSet::builtin`
alongside the existing five (so they are available to every graph by default,
and to `DEFAULT_TOML`'s agent node which grants all tools).
- `grep { pattern, path?, cwd? }` — recursive regex search under `cwd` (default
  workspace), returning matching `path:line:content` lines, capped at 64 KiB
  like `bash` output. Respects the same `PathScope`/workspace confinement as
  `fs_read` (no reads outside the workspace).
- `glob { pattern, cwd? }` — list files under `cwd` (default workspace) matching
  a glob pattern, one per line, capped at 64 KiB. Same workspace confinement.
Both are read-only (no `FS_WRITE`/`PROC` needed beyond what `fs_read` uses).

### Non-Functional

- **N1**: No new dependencies. `process_group` is std; `killpg` is `libc`
  (already a dependency of the workspace — verify before adding). Glob matching
  uses `glob` crate if already present, else a hand-rolled matcher — do NOT add
  a new crate without checking `Cargo.toml` first.
- **N2**: `#![forbid(unsafe_code)]` holds except pii-hw/pii-memory. `process_group(0)`
  is safe std; if `killpg` needs `unsafe`, isolate it in a single `#[allow(unsafe_code)]`
  helper in `pii-core` with a doc comment explaining why (the only sanctioned
  site), mirroring the existing "one `Command` site" discipline.
- **N3**: The 450-line file cap holds. If `ctx.rs` or `bash.rs` grows past it,
  extract helpers into a new module (e.g. `pii-core/src/proc.rs`).
- **N4**: `cargo xtask deps-check` and `cargo xtask net-check` must still pass —
  no new socket-opening code, no new dependency edges.

## Architecture

```
AgentNode::run (agent.rs)
  └─ ChatRequest { model, messages, tools, max_tokens, temperature, num_ctx }
       └─ OllamaProvider::build_request  →  { model, messages, tools,
                                              temperature, num_predict, num_ctx,
                                              stream: false }
BashTool::call (bash.rs)
  └─ ProcCap::run (ctx.rs)
       └─ run_child: Command::new(argv0)
            .process_group(0)          // F3: own process group
            .env_clear().envs(ALLOWLIST)  // F2: scrubbed env
            .spawn()
            └─ try_wait loop: polls ctx.check() every 10ms  // F3: cancel
                 on timeout → killpg(child.pid)             // F3: kill group
GrepTool / GlobTool (pii-tools/src/{grep,glob}.rs)
  └─ workspace-scoped read, 64 KiB cap
```

**Key decisions:**
- **Env scrubbing via allowlist, not deny-list.** A deny-list of `*_API_KEY`
  names is fragile (a future secret var not on the list leaks). An allowlist of
  the ~7 vars a shell needs is exhaustive and future-proof. Rejected: inheriting
  and filtering — too easy to miss one.
- **`process_group(0)` + `killpg` over `child.kill()`.** `child.kill()` sends
  SIGKILL to the direct child only; grandchildren survive. A process group is
  the correct unit of "this tool call's work."
- **`num_ctx` default 8192.** Matches a reasonable local-model context; the
  compaction gate (4.1b) will later use it as its budget reference.
- **`grep`/`glob` as real tools, not bash wrappers.** A bash wrapper would
  re-inherit the scrubbed-env problem and burn a shell spawn per call. Direct
  Rust implementations are faster and stay inside the capability model.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `ollama_request_forwards_provider_options` (pii-model unit) | `ChatRequest { max_tokens: 2048, temperature: 0.7, num_ctx: 16384 }` | `build_request` JSON body contains `"temperature":0.7`, `"num_predict":2048`, `"num_ctx":16384`, `"stream":false` |
| `ollama_request_defaults_num_ctx` (pii-model unit) | `ChatRequest` with `num_ctx: 0` | body contains `"num_ctx":8192` (the default) |
| `run_child_scrubs_environment` (pii-core unit) | a child that prints `env` | output contains `PATH`/`HOME` but NOT a sentinel `TEST_SECRET=xyz` set in the parent env |
| `run_child_kills_process_group_on_timeout` (pii-core unit) | a child that spawns a grandchild sleeping 60s, timeout 200ms | after timeout, both the child and grandchild are dead (grandchild pid no longer alive); `ProcOutput.timed_out == true` |
| `run_child_returns_cancelled_when_ctx_cancelled` (pii-core unit) | a child sleeping 60s, ctx cancelled after 100ms | `run_child` returns `PiiError::Cancelled`; child killed |
| `grep_tool_finds_matches_in_workspace` (pii-tools unit) | a temp workspace with a file containing `needle`, `grep { pattern: "needle" }` | result contains `path:line:content` with the matching line |
| `grep_tool_denies_outside_workspace` (pii-tools unit) | `grep { pattern: "x", cwd: "/etc" }` | `PiiError::Denied` (outside workspace) |
| `glob_tool_lists_matching_files` (pii-tools unit) | a temp workspace with `a.rs`, `b.rs`, `c.txt`, `glob { pattern: "*.rs" }` | result lists `a.rs` and `b.rs` (one per line), not `c.txt` |
| `glob_tool_denies_outside_workspace` (pii-tools unit) | `glob { pattern: "*", cwd: "/etc" }` | `PiiError::Denied` |

The **acceptance test** is `run_child_scrubs_environment` — it is the one a
plausible implementation gets wrong by filtering only `*_API_KEY` names instead
of using an allowlist, or by forgetting `spawn_child` (the second Command site).
The spec requires BOTH `run_child` and `spawn_child` to scrub; the test must
cover both paths.

## Exit Criteria

- [ ] `cargo build --workspace --all-targets`
- [ ] `cargo test -p pii-model` — all pass (incl. new provider-options tests)
- [ ] `cargo test -p pii-core` — all pass (incl. new env-scrub + process-group + cancel tests)
- [ ] `cargo test -p pii-tools` — all pass (incl. new grep/glob tests)
- [ ] `cargo test --workspace` — 0 failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors
- [ ] `cargo fmt --all --check` — clean
- [ ] `cargo xtask lint-files` — 0 hard errors
- [ ] `cargo xtask deps-check` — pass (no new dependency edges)
- [ ] `cargo xtask net-check` — pass (no new socket-opening code)
- [ ] `cargo test -p pii-core run_child_scrubs_environment` — the F2 env-scrub proof (a sentinel secret set by the test harness must NOT leak into the child's env). This is the authoritative machine-checkable proof of F2; the old daemon-dependent `pii -p` check is dropped because it needs a live daemon whose single-writer `store.redb` conflicts with concurrent pidag validate runs.

## Guardrails

- Do NOT add a new crate dependency without checking `Cargo.toml` first — the
  workspace has strict `deps-check` invariants. Prefer std + existing deps.
- Do NOT weaken the deny-list or the `PathScope`/workspace confinement — the
  new `grep`/`glob` tools must respect the SAME workspace boundary as `fs_read`.
- Do NOT change `stream` to `true` — streaming is 4.1b and needs a streaming
  transport, out of scope here.
- Do NOT touch `specs/` — you are the workhorse, not the planner.
- Do NOT commit. Checkpoint-commit clean-building work between exit criteria
  if you hit a tool-call/iteration budget mid-task — do not lose work.
- If `killpg` requires `unsafe`, isolate it in ONE `#[allow(unsafe_code)]`
  helper with a doc comment; do not sprinkle `unsafe` through the file.
