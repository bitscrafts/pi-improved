# pi-improved — spec-19: phase4-web-viewer

- **Project**: `/projects/pi-improved`
- **Priority**: P4.6 — observability: a browser window into the live daemon's runs, sessions, approvals, heals, memory. Worth it for observability, not a second chat UI (opus plan).
- **Status**: SPECIFIED — not yet dispatched · *(lifecycle: SPECIFIED → IN PROGRESS
  on dispatch → IMPLEMENTED — awaiting review → DONE after the orchestrator
  commits and updates HANDOFF.md)*
- **Source**: Phase 4.6 - pii web viewer (2026-09-07), building on the spec 12 draft
- **Depends-On**: spec 16 (4.2) — `session.list` dispatch arm landed; the viewer's session view depends on it.

---

## Verified Premises

- `crates/pii-proto/src/client.rs:68-164` — `Client` provides request/response demux + an `events()` broadcast receiver. The ENTIRE client half exists and is used by pii-cli/pii-tui.
- `crates/pii-server/src/uds.rs` — `client_connect(&sock)` + `runtime_dir(instance)` resolve the daemon socket path exactly like the CLI.
- `crates/pii-server/src/dispatch.rs:83-90` — `Method::SessionList` dispatch arm now exists (landed in spec 16 part2). The viewer's session list depends on it.
- `crates/pii-server/src/dispatch.rs:43` — `sys.status` exists; `run.list` (dispatch.rs:109/185), `run.trace` (dispatch.rs:301), `event.subscribe` (subscribe.rs), `session.state`, `approval.list`, `heal.list`, `memory.search` all exist.
- `crates/pii-net/src/service.rs` — the daemon's UDS JSON-RPC is the only way to reach the store (redb single-connection); the viewer MUST proxy the daemon, not read the store file.
- `crates/pii-cli/src/cmd_ctl.rs:60-71` — `print_result` shows the CLI's RPC-call pattern the web server will reuse.
- Workspace has NO web dependency yet (no axum anywhere). The opus plan + spec 12 guardrail: keep axum in a NEW `pii-web` crate, not the workspace root or pii-cli.

---

## Overview

The opus Phase-4 review's **P4.6**: *"`pii web` read-only viewer. Worth it for observability, not a second chat UI. The architecture in the draft is right (redb single-connection → proxy the daemon's UDS JSON-RPC over loopback HTTP; separate `pii-web` crate keeping axum out of core). Two amendments: sequence after P4.2 (a run browser on top of a `run.list` that returns `[]` and a missing `session.list` ships a viewer whose main job is displaying incorrect state); prefer SSE over polling for the live view."*

This spec builds the read-only web viewer:
1. A new `pii-web` crate (axum server) that proxies the daemon's UDS JSON-RPC over loopback HTTP.
2. An embedded vanilla-JS SPA (pidag-ui pattern) with views: daemon status, run list, run trace, session transcript, approvals, heals, memory.
3. `session.list` already landed (spec 16); the viewer uses it.

**Scope is VIEWER-ONLY**: no prompt input, no run creation, no mutation. Chat stays in the TUI / `pii -p`.

---

## Requirements

### Functional

- **R1 (pii-web crate)**: a new `crates/pii-web` crate (axum server) with a `pii web [--port 4602] [--host 127.0.0.1] [--instance default]` subcommand wired into `pii-cli`. It resolves the daemon socket via `uds::runtime_dir` + `client_connect`, verifies the daemon with `sys.status`, and refuses to start with a clear error if unreachable.
- **R2 (HTTP proxy)**: the server proxies daemon RPCs over HTTP: `GET /api/status` → `sys.status`, `GET /api/runs` → `run.list`, `GET /api/runs/<id>/trace` → `run.trace`, `GET /api/sessions` → `session.list`, `GET /api/sessions/<id>` → `session.state`, `GET /api/approvals` → `approval.list`, `GET /api/heals` → `heal.list`, `GET /api/memory?q=` → `memory.search`. One UDS client per browser request (or one shared client multiplexed by request id — `Client` already demuxes by id).
- **R3 (SSE live view)**: `GET /api/events` streams `event.subscribe` pushes over SSE (Server-Sent Events) for the live run view — the opus plan's amendment over polling. The `Client::events()` broadcast receiver maps to SSE `data:` frames.
- **R4 (embedded SPA)**: an embedded `index.html` (include_str!), hash-routed, no external assets, dark theme, plain-ASCII rendering. Views: `#/` (status + run list), `#/run/<id>` (trace, SSE-live while Running), `#/session/<id>` (transcript), `#/approvals`, `#/heals`, `#/memory`. A "daemon: ok / unreachable" header indicator.
- **R5 (loopback-only)**: default bind 127.0.0.1, port 4602. No auth — loopback-only, mirroring pidag-ui. `--host 0.0.0.0` exists only as the documented escape hatch.

### Non-Functional

- **N1**: VIEWER-ONLY — no write RPC proxied, no session.create/prompt from the web UI.
- **N2**: axum confined to `pii-web` — the workspace root and pii-cli stay lean (deps-check invariant).
- **N3**: the quality gate stays green; the test count may only go up.
- **N4**: `xtask` gates stay green (deps-check must allow axum under pii-web only).

---

## Architecture

```
Browser (vanilla-JS SPA, embedded via include_str!)
   |
   | HTTP GET /api/*  (127.0.0.1:4602 only)
   v
pii web  =  axum server (crates/pii-web)  -- one UDS client per request
   |        (pii_proto::client::Client demuxes by id)
   v
daemon UDS socket (pii-server) — sys.status, run.list, run.trace,
      session.list, session.state, approval.list, heal.list, memory.search,
      event.subscribe (SSE)
```

**Key decision — proxy the daemon, never read the store file.** redb is single-connection; a second process cannot open `store.redb` while the daemon runs. The viewer proxies the daemon's UDS JSON-RPC over loopback HTTP — the same relationship pii-cli/pii-tui have, but for a browser.

**Key decision — SSE over polling for the live view.** The opus plan's amendment: `event.subscribe` maps almost one-to-one onto SSE, and it makes P4.1's streaming deltas visible in the browser. Polling stays for the non-live lists (status, run list).

**Key decision — axum in a separate `pii-web` crate.** The workspace has no web dep; adding axum workspace-wide would bloat every binary. A dedicated crate keeps core lean and matches the deps-check discipline.

**What this spec is not**: a chat UI, run creation, approval resolution, TLS (loopback-only), session persistence (daemon-memory-only, pre-existing).

---

## TDD Contract

| id | test | given | expects |
|----|------|-------|---------|
| R1 | `web_refuses_to_start_without_daemon` (pii-web unit) | point the server at a nonexistent socket | startup returns a clear error mentioning the socket path; exit non-zero |
| R2 | `web_proxies_sys_status` (pii-web integration) | daemon running, GET /api/status | HTTP 200, body contains the daemon's model/version fields |
| R2 | `web_run_trace_returns_journal` (pii-web integration) | run a real graph, GET /api/runs/<id>/trace | HTTP 200 with the terminal event present, seqs in order |
| R2 | `web_sessions_view_lists_session` (pii-web integration) | create + prompt a session, GET /api/sessions | the session appears with its metadata |
| R3 | `web_events_streams_sse` (pii-web integration) | subscribe to /api/events, trigger a run | SSE frames arrive with the run's events |
| R4 | `spa_has_no_external_asset_references` (static check) | grep the embedded HTML for `http://`, `https://`, `<script src` | no matches (self-contained) |
| R5 | `web_binds_loopback_by_default` (pii-web unit) | start with no --host | binds 127.0.0.1, not 0.0.0.0 |

**R2 (`web_proxies_sys_status`) is the acceptance test.** The requirement a plausible implementation gets wrong: it reads the store file directly (fails on redb single-connection) or it doesn't proxy the daemon at all. The test asserts a real HTTP round-trip through the daemon proxy.

---

## Exit Criteria

- [ ] `cargo build --workspace --all-targets` — compiles (R1-R5)
- [ ] `cargo test -p pii-web` — all pass (R1-R5)
- [ ] `cargo test -p pii-server session_list::` — session.list still passes (dep)
- [ ] `cargo test --workspace` — 0 failures (N3)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors (N3)
- [ ] `cargo fmt --all --check` — clean (N3)
- [ ] `cargo xtask lint-files` — 0 hard errors (N3)
- [ ] `cargo xtask deps-check` — pass (N2, N4)
- [ ] `cargo xtask net-check` — pass (N4)
- [ ] Manual: `pii web` starts, http://127.0.0.1:4602 shows the daemon status card and run list; opening a run's trace shows seq-ordered rows; stopping the daemon flips the header to "unreachable"

**Prose criteria**:

1. Test counts pasted raw, one line per binary, **unsummed**.
2. The viewer must proxy the daemon (R2), not read the store file — a direct-redb implementation is a FAIL.
3. axum must be confined to `pii-web` (N2) — deps-check must pass.

---

## Guardrails

- **G1 — do NOT edit this spec.** If it is wrong, STOP and report it.
- **G2 — do NOT commit.** Leave work in the working tree.
- **G3 — do NOT weaken, skip or delete an existing test.**
- **G4 — do NOT regenerate a pinned fixture.**
- **G5 — no hardcoded absolute paths.** Test artefacts under `_tmp/`.
- **G6 — report raw output, never summed totals.**
- **G7 — VIEWER-ONLY.** No prompt input, no run/approval/heal/memory mutation, no write RPC proxied. The moment the web UI can start runs, it needs auth + a real security story (separate spec).
- **G8 — do NOT add axum to the workspace root or pii-cli.** Keep it in `pii-web` only.
- **G9 — loopback-only default, no TLS.** A localhost dev viewer does not need it.

### Error handling expectations

- A daemon that goes down mid-session must flip the header to "unreachable" with the error, not crash the server.
- A proxy RPC failure must return an HTTP error with the daemon's error text, not a silent empty body.
- SSE must close cleanly when the daemon disconnects.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/pii-web/Cargo.toml` | new crate: axum, tokio, pii-proto (client), pii-server (uds) |
| `crates/pii-web/src/main.rs` | `pii web` server + CLI |
| `crates/pii-web/src/proxy.rs` | UDS JSON-RPC → HTTP proxy (R2) |
| `crates/pii-web/src/sse.rs` | event.subscribe → SSE (R3) |
| `crates/pii-web/src/index.html` | embedded SPA (R4) |
| `crates/pii-cli/src/main.rs` | wire `pii web` subcommand (R1) |
| `Cargo.toml` | add `crates/pii-web` to workspace members |
| `xtask/src/deps.rs` | axum-confined-to-pii-web invariant (N2) |

**Not modified**: `crates/pii-server/` (no daemon change — session.list already landed), `crates/pii-proto/` (no wire change), `crates/pii-net/` (no socket change).
