# Spec: `pii web` — read-only viewer web UI (daemon-proxied, pidag-ui-style)

**Project**: <repo>
**Phase**: post-3.6 — developer-ergonomics feature (independent of
`specs/09-global-memory.md`, `specs/10-graph-visualize.md`,
`specs/11-journal-hardening.md`)
**Status**: DRAFT — spec only, no implementation authorized yet. User
decision (2026-09-06, clarify): build the read-only VIEWER first ("ships
fastest, zero risk to core"), design base = pidag-ui (the proven
pure-Rust embedded-SPA trace UI in the same family, live at
http://the host:4601). Implementation deferred to a future pidag run on
greenlight.

## Overview

dsh ships a browser UI as its default experience; pi is CLI-first. This
spec adds a lightweight READ-ONLY web viewer for the things pi already
records — run journals (the append-only event logs with seq numbers),
graph topology, session transcripts, approvals, heals, memory — so a
human can watch and debug a live pi daemon from a browser without
touching the TUI, SSH, or raw JSON-RPC.

Scope is deliberately VIEWER-ONLY: no prompt input, no run creation, no
approval resolution, no mutation of any kind. Chat stays in the TUI /
`pii -p`; this is the "what is the daemon doing, and what did it do"
window. (A future spec can add a live console; the data plumbing built
here — daemon proxying + event streaming — is exactly its foundation.)

## Architecture (why NOT direct redb reads)

pi's daemon owns `store.redb` with `redb::Database::create()`, which
fails if a second process opens the same file (single-connection model).
A viewer CANNOT read the store file directly while the daemon runs.
Design base instead: **proxy the daemon's existing Unix-socket JSON-RPC
over HTTP** — the same relationship `pii-cli`/`pii-tui` have to the
daemon, but for a browser.

```
Browser (vanilla-JS SPA, embedded via include_str!)
   |
   | HTTP GET /api/*  (127.0.0.1:PORT only — no TLS exists, no auth,
   |                  so loopback-only like pidag-ui's default)
   v
pii web  =  axum server (NEW binary/crate)  -- one UDS client per browser
   |        request, or one shared client multiplexed by request id
   |        (pii_proto::client::Client already demuxes by id)
   v
daemon UDS socket (pii-server) — sys.status, session.state, run.list,
      run.trace, event.subscribe, approval.list, heal.list, memory.search
```

Why this is sound:
- `pii_proto::client::Client` (feature `client`) already provides
  request/response demux + an `events()` broadcast receiver for live
  `EventPush` — the ENTIRE client half exists and is used by pii-cli,
  pii-tui, pii-server tests.
- `pii_server::uds::client_connect(&sock)` + `uds::runtime_dir(instance)`
  already resolve the socket path exactly like the CLI does.
- redb single-connection is sidestepped entirely; no store changes, no
  daemon changes beyond one method handler (below).
- The viewer works against ANY reachable daemon (local dev, the the host
  deployment) with zero file access.

### Data surface (all READ-ONLY RPC, all already implemented except one)

| View | RPC | Status |
|---|---|---|
| Daemon status | `sys.status` | exists (dispatch.rs:43) |
| Run list | `run.list` | exists (dispatch.rs:109/185) |
| Run detail + journal trace | `run.trace` | exists (dispatch.rs:301) — returns the full seq/event journal |
| Live event stream | `event.subscribe` (+ `event.unsubscribe`) | exists (subscribe.rs) — replay-since + live, bounded queue, `Lagged` instead of drops |
| Session transcript | `session.state` | exists |
| Session list | `session.list` | **GAP**: declared in the wire protocol (crates/pii-proto/src/method.rs:35) but NO dispatch arm exists — falls through to `MethodNotFound` (crates/pii-server/src/dispatch.rs:141). v1 workaround: the SPA tracks sessions it has seen (create/prompt replies carry session ids); PLUS implement the handler (small, see Requirements W2) |
| Approval queue | `approval.list` | exists (methods.rs) |
| Heal ledger | `heal.list` | exists (methods.rs) |
| Memory browse/search | `memory.search` | exists (methods.rs) |

Note: sessions live in daemon MEMORY (`Engine::sessions`), not in the
store — sessions vanish on daemon restart (pre-existing limitation,
documented in HANDOFF). The viewer shows what the daemon currently
holds; it does not promise session history across restarts.

## Requirements

### Functional

#### W1 — Server & CLI
- New subcommand `pii web [--port 4602] [--host 127.0.0.1] [--instance
  default]` wired into `pii-cli` (cmd_graph-style module; new
  `cmd_web.rs`), OR a separate `pii-web` binary crate — decision left
  to the implementer with a strong lean toward a separate small crate
  `crates/pii-web` (axum 0.8 is a heavy dep; the pii-cli binary stays
  lean; pidag-ui precedent puts the UI server in its own module of the
  same crate, but pi's workspace has no web dep at all yet — see
  Guardrails for the no-workspace-axum rule). The command resolves the
  daemon socket exactly like every other pii subcommand (uds::runtime_dir
  + client_connect), verifies the daemon is reachable with `sys.status`,
  and refuses to start with a clear error if not (the viewer is
  useless without the daemon — no store file fallback).
- Default bind 127.0.0.1, port 4602 (pidag-ui owns 4601). No auth —
  loopback-only, mirroring pidag-ui's safety stance. `--host 0.0.0.0`
  exists only for the same "you understand the implications" escape
  hatch pidag-ui documents.
- Graceful shutdown on SIGINT/SIGTERM (axum `with_graceful_shutdown`).

#### W2 — Daemon side: implement `session.list` (small, required)
- Add the dispatch arm for `Method::SessionList` (protocol name
  `session.list`, pii-proto/src/method.rs:35 already declares it) in
  crates/pii-server/src/dispatch.rs next to the other session methods:
  list the live `Engine::sessions` (id, created time, message count,
  status) as JSON. The Engine already owns the map; this is ~15 lines +
  a params/result type in pii-proto (SessionListResult) + 1 unit test
  (session created -> appears in list; unknown method regression
  covered by the existing dispatch tests). NO other daemon changes.
- This is the ONLY server change in this spec, and it is additive —
  the wire name already exists, so no protocol version bump.

#### W3 — SPA views (vanilla JS, embedded, pidag-ui pattern)
One embedded `index.html` (include_str!), hash-routed, no external
assets, dark theme, plain-ASCII/ANSI-colored log rendering consistent
with the TUI's no-emoji convention:
- `#/` — daemon status card (model, uptime, store paths from
  `sys.status`) + run list table (id, graph, status, seq, duration)
  polling every ~2 s (pidag-ui's proven poll model; live push via
  `event.subscribe` is a v2 nicety — see Guardrails).
- `#/run/<id>` — the trace view: the run's full journal rendered as
  seq-ordered rows (the same vocabulary `pii graph trace` prints:
  Started/TickStarted/NodeStarted/NodeFinished/NodeFailed/NodeDead/
  NodeCancelled/Suspended/Resumed/Finished — plus NodeInterrupted once
  spec 11 lands), each row with its hex seq, node id, port, and the
  same role-coloring the TUI uses (user/model/thinking/system/error).
  Auto-poll every ~2 s while the run is Running/Suspended; stop when
  terminal.
- `#/session/<id>` — session transcript from `session.state`
  (WireMessage list, role-colored).
- `#/approvals`, `#/heals`, `#/memory` — the read-only lists from the
  corresponding RPCs (approval.queue, heal ledger entries, memory
  search box that calls memory.search with the typed query).
- Header nav between views; a "daemon: ok / unreachable" indicator that
  turns red with the last error when polls fail (the daemon is the
  single point of failure; when it is down the viewer says so plainly).
- All rendering of journal events reuses the SAME logic the TUI and
  `pii graph trace` use where it can be shared: if a shared
  pretty-printer exists or is trivially extractable (pii-cli trace
  formatting), share it; otherwise duplicate the small format map in
  the SPA JS and note the drift risk in the code (no cross-crate
  plumbing for strings in v1).

### Non-Functional
- Loopback-only default; no TLS anywhere (the B4 pii-net gap is NOT
  touched by this spec — pii web is a localhost dev tool by design).
- Zero changes to the run engine, store, journal, or agent — viewer +
  one additive session.list handler only.
- Polling (not WebSocket/SSE) in v1 for live views — pidag-ui's proven
  pattern, one less moving part; event.subscribe plumbing exists for
  the future live console spec.
- Vanilla JS + inline CSS in the embedded HTML — no Node toolchain, no
  build step, no external CDN (matches pidag-ui_assets/index.html).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `session_list_returns_created_sessions` (pii-server dispatch unit) | create a session, call `session.list` | the session id appears with its metadata; empty store -> `{"sessions": []}` |
| `web_refuses_to_start_without_daemon` (pii-web unit) | point the server at a nonexistent socket | startup returns a clear error mentioning the socket path; exit non-zero |
| `web_proxies_sys_status` (pii-web integration: spawn daemon on a test socket via the existing testkit, start web on an ephemeral port, GET /api/status) | daemon running | HTTP 200, body contains the daemon's model/version fields |
| `web_run_trace_returns_journal` (same harness) | run a real graph to completion, GET /api/runs/<id>/trace | HTTP 200 with the terminal event present and seqs in order |
| `web_sessions_view_lists_session` (same harness) | create + prompt a session, GET /api/sessions | the session appears with its messages count |
| `spa_has_no_external_asset_references` (static check) | grep the embedded HTML for `http://`, `https://`, `<script src` | no matches (self-contained; fonts/icons are system/ANSI, no CDN) |
| `health_endpoint_reports_daemon_down` (pii-web unit) | start web against a socket that exists then disappears | /api/health returns `{"daemon":"down"}` with the error text, not a crash |

## Exit Criteria
- [ ] `cargo test -p pii-server session_list::` — pass (W2 handler)
- [ ] `cargo test -p pii-web` — all pass
- [ ] `cargo test --workspace` — 0 failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 errors
- [ ] `cargo fmt --all --check` — clean
- [ ] Manual live check against the the host daemon: `pii web` starts,
      http://127.0.0.1:4602 shows the daemon status card and the run
      list from the live deployment's store; opening a recent run's
      trace shows seq-ordered rows; stopping the daemon flips the
      header indicator to "daemon unreachable" with the error
- [ ] All `xtask` gates pass (lint-files/deps-check/net-check/
      reachability-check/doc-drift-check)

## Guardrails
- VIEWER ONLY — no prompt input, no run/approval/heal/memory mutation,
  no session.create/prompt from the web UI, no write RPC proxied. The
  moment the web UI can start runs, it needs auth + a real security
  story; that is a separate spec. (The one exception is the additive
  read-only `session.list` handler in W2.)
- Do NOT add axum (or any web dep) to the pii-cli binary or the
  workspace root — keep it in the new pii-web crate only, so the core
  binaries and the workspace dep graph stay lean. pidag-ui keeps its
  axum inside the pidag crate; pi-improved has NO web dep yet and
  should not acquire one workspace-wide for a viewer.
- Polling in v1, not WebSocket/SSE — pidag-ui's proven model; the
  event.subscribe live-stream plumbing is already there and stays for
  the future live-console spec, but v1 must not build the live path.
- Loopback-only default and NO TLS work — the B4 pii-net TLS gap is
  explicitly out of scope; a localhost dev viewer does not need it and
  must not become the excuse to build it prematurely.
- Do NOT touch specs/09, 10, or 11 (global memory, graph show, journal
  hardening) — independent tracks; this spec's trace view should
  render `NodeInterrupted` when spec 11 ships it, but does not depend
  on it.
- Sessions are daemon-memory-only (pre-existing limitation) — do NOT
  build session persistence into this spec; note it in the UI ("sessions
  are live — restarting the daemon clears them") and move on.
