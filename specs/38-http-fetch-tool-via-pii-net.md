# Spec: HTTP Fetch Tool via pii-net Chokepoint

## Overview
Adds an HTTP(S) fetch tool for limited external network access by workers, routed exclusively through the pii-net chokepoint with deterministic logging and filtering. Enforces the single-socket invariant, audit logging, and future policy controls. Does NOT permit arbitrary unrestricted fetch—only credential-free, explicit external GET (no mutation, no vendor cloud APIs).

**Project**: /projects/pi-improved
**Phase**: 5.1 - Secure External Fetch Service

## Requirements
### Functional
- Workers may fetch external HTTP(S) resources using a dedicated pii-http-fetch tool.
- All requests are routed through the pii-net chokepoint (no direct access, no vendor cloud APIs).
- All requests and responses are logged with deterministic replay (inputs/outputs).
### Non-Functional
- Enforces the single-socket rule: _all_ network access by workers passes through chokepoint.
- PII/secret embargo: tool exposes only credential-free public fetch (no keys, secrets, cookies, or auth headers).
- Explicitly blocks POST/PUT/DELETE and upload; _GET only_.
- Deterministic: duplicate inputs yield identical outputs (modulo source volatility).

## Architecture
+----------+    GET    +----------+    GET    +------+
|  worker  |---------> | fetcher  |---------> |Web/ex|
+----------+           +----------+           +------+
                      (pii-net)

## TDD Contract
| Test Name                | Input                    | Expected Output                |
|--------------------------|-------------------------|-------------------------------|
| fetch_basic_get          | http://example.com      | 200/HTML body, audit log      |
| fetch_https              | https://example.org     | 200/response, deterministic   |
| fetch_post_blocked       | POST X                  | error: only GET permitted      |
| fetch_with_headers       | URL + headers           | error: headers blocked         |

## Exit Criteria
- [ ] `cargo test -p pii-http-fetch` (all green; proves basic/prohibited cases)

## Amendment (2026-09-16, architect)
Original R2 (`grep -qE '^FEED: GET ' /tmp/pii-net-audit.log`) is dropped, not
fixed-in-place: it was unsatisfiable for two stacked reasons, not one. (1) it
hardcoded a `/tmp/` path no DAG node ever populates, violating this
workspace's own "test artifacts never write to `/tmp/`" convention; (2)
deeper — `HttpFetchTool` is a complete, unit-tested tool
(`crates/pii-http-fetch/`, 8 tests covering GET/HTTPS/POST-blocked/
headers-blocked/no-grant-denied, all routed through the `pii-net`
chokepoint) but is not registered anywhere reachable (`pii-tools::ToolSet`,
`pii-server`, `pii-cli` all have zero references to it). There is no real
end-to-end code path yet for a file-backed audit log to prove itself
against — R2 was asking for evidence of an integration this spec never
built. `AuditLog::to_file`/`AuditSink::File` exist and are exercised
directly by `audit_file_sink_is_deterministic` (unit-level, in-memory vs.
file byte-parity), which is the honest level of coverage this spec's scope
supports.

Registering `HttpFetchTool` into `pii-tools::ToolSet` (so a real agent run
can invoke it and produce a genuine file-backed audit trail) is separate,
follow-on work — a new spec, not a fix to this one's R2.

## Guardrails
- Do NOT permit secrets/keys/creds in tool API
- Do NOT allow POST/PUT/DELETE or uploads
- All network initiated by a worker _must_ route through chokepoint
- All logs are deterministic and auditable
