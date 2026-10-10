# Spec: 111 — provider kinds: an OpenAI-compatible kind, `anthropic`, `gemini`

**Project**: /projects/pi-improved
**Phase**: Stage A of the vision plan (`_docs/06-research/057-vision-implementation-plan-2026-10-10.md`) —
finish the gateway. First row of the order, because every later stage answers through a provider.

## What exists (measured 2026-10-10; do not re-derive)
- `crates/pii-model/src/lib.rs:241` `provider_from_spec(spec) -> Arc<dyn ModelProvider>` handles
  `scripted:<path>`, `llamacpp` (feature-gated, off by default) and refuses `ollama:` (it has no transport).
- `crates/pii-model/src/lib.rs:271` `resolve_with_transport(spec, transport: Arc<dyn pii_core::HttpTransport>)`
  is the entry point a real caller uses — it is the ONE place that can honour a provider that needs HTTP.
- The transport seam is `Arc<dyn pii_core::HttpTransport>`, whose implementation (`pii-net`'s `NetService`)
  carries the **egress allowlist** (`http:<host>`) — so a new provider inherits the existing policy instead of
  inventing one. `crates/pii-model/src/ollama.rs` is the worked example (request build → send → parse →
  `PiiError`), and `crates/pii-model/tests/ollama_http.rs` is the offline test pattern.
- The failover classifier already exists and is pure: `pii_abi::failover::classify(status, message, origin)`.

## Requirements
R1. **Three new kinds, all through the transport seam:**
    - `openai:<model>` — OpenAI-compatible `POST {base}/chat/completions`. **The base URL is configurable**
      (env/config), because that single kind is what makes the vision's "~20 providers" true cheaply:
      most vendors expose this shape. State the env/config key and its default.
    - `anthropic:<model>` — the Messages API (`POST /v1/messages`, `x-api-key`, `anthropic-version`).
    - `gemini:<model>` — `generateContent` (`key` as a query param or header — say which and why).
    Each is a module in `pii-model`, reachable from **both** `provider_from_spec` (where a transport is not
    needed — refuse with a clear message like `ollama:` does) and `resolve_with_transport` (where it is).
R2. **The egress path stays governed**: a provider must be constructed with the SAME `Arc<dyn HttpTransport>`
    the engine already wires, so the `http:<host>` allowlist applies. Say how the host is derived from the
    base URL and what happens when the allowlist does not include it (the denial must be reported, not
    swallowed — see R4).
R3. **Streaming: state it per kind, honestly.** If a kind streams (SSE), parse it and emit a partial-token
    event through `pii-observe`; if it does not, return a clear "streaming unsupported for this kind" rather
    than silently buffering the whole body and pretending. Do not add a new dependency for SSE without saying
    which and why (a line-oriented SSE reader is small).
R4. **Errors map to the existing shape AND are observable**: HTTP status + provider error body →
    the existing provider-failure type, fed through `pii_abi::failover::classify` so retryability keeps one
    home; and every failure the provider owns (auth failure, rate limit, malformed body, egress denial)
    emits a NAMED event registered in `pii-observe::names` + `ALL` (the completeness rule enforces the
    registry). Never log the request headers — they carry keys.
R5. **Tests are OFFLINE.** One test per kind against a mock/recorded `HttpTransport` (the `ollama_http.rs`
    pattern), covering at minimum: the exact REQUEST SHAPE (path, method, the header names present, the body
    JSON), a successful response (text + token counts where the API reports them), an error response
    (401 and 429 → the expected error kind + the classifier's verdict), and a malformed body. Plus ONE live
    smoke test per kind marked `#[ignore]`, documented as needing network + a key, and never run by the gate.
    **The exact request JSON the provider builds must be pasted in the report** — that is the artefact a
    reviewer reads.
R6. **`provider_from_spec`'s unknown-kind message must list every supported kind** (today it names only two).
R7. **A config sample** for each kind in `examples/` (or the existing production sample extended), with the
    env var names for keys/URLs, and a line saying a key must never be logged (the spec-107 canary test
    covers the log/ndjson/trajectory paths — extend it to a provider path if the key could reach a field).

## Exit Criteria
- [ ] three kinds reachable from both entry points (paste `provider_from_spec` and `resolve_with_transport`
      behaviour for each, including the transport-less refusal)
- [ ] the request-shape JSON pasted for each kind (the mock transport's captured request)
- [ ] the error mapping table pasted: status/body → error → classifier verdict, with the 401 and 429 rows real
- [ ] streaming stated per kind, with the partial-token event pasted if a kind streams
- [ ] the new name consts listed and present in `ALL` (the lint's completeness rule passes)
- [ ] `cargo test -p pii-model` exits 0 with the offline tests; the ignored live tests are named
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability` and `PASS: coverage-floor` (or the
      documented `QG_SKIP_COVERAGE=1` form, said aloud); `fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] `_docs/01-overview/001-pii-vision.md`'s provider line is updated to the measured count (the "~20" claim
      becomes a number with the kinds named)
- [ ] the report lists every file changed, every new env/config key, and every new dependency (or states none)

## Guardrails
- **No live network in any gate test.** The gate must pass on a host with no internet.
- Never log a key, a header, or an env value; a denial or an auth error names the HOST, never the credential.
- Do not invent token counts: if a provider does not report them, say so rather than estimating.
- Reuse `ollama.rs`'s shape; do not fork a second HTTP style into the crate.
- Scratch under `/projects/_tmp/`; the volume is tight (19G free) — keep it small and delete your own.
  No daemon left running.
- Do NOT commit; leave the tree for the orchestrator.

## Orchestrator verification (2026-10-10) — one real gap found and closed
**My controls:**
- `cargo test -p pii-model` → 20 unit + 5 `ollama_http` + 19 `provider_kinds` pass, 3 `live_smoke` ignored ✓
  (so the gate never touches the network).
- **The engine's grant is per host, so the allowlist applies end to end**: `crates/pii-server/src/lib.rs:182-201`
  takes `pii_model::egress_host(model)` and asks for `http:<host_only>` before `resolve_with_observer`.
- **What `provider_from_spec` says for a networked kind**: `Err("<kind>: needs a transport — call
  <kind>::provider_from_env or resolve_with_transport")`, and the unknown-kind error lists all six kinds ✓.

**THE GAP (found by my mutation, not by the report): the three auth/version headers were asserted by
PRESENCE only.** Renaming the pinned `anthropic-version` from `2023-06-01` to `2023-01-01` left all 19 tests
GREEN — the report's request-shape table claimed that header as evidence, but nothing checked its value, and
the same weakness covered `Authorization` (scheme + key) and `x-goog-api-key`. A protocol version or an auth
scheme can be wrong and every test still passes: exactly "a check that passes while the live path is broken".
**Fixed by the orchestrator** in `crates/pii-model/tests/provider_kinds.rs`: the mock gained a case-insensitive
`header(name)` accessor and the three tests now assert VALUES — the pinned version against the literal, the
`Bearer <key>` scheme, and the gemini key header. Controls re-run: renaming the version → `left: Some("2023-01-01")`
vs `right: Some("2023-06-01")` FAILS; `Bearer`→`Basic` → FAILS; restored → 19 pass.

**Honest limits carried:** `anthropic`/`gemini` streaming is refused with a clear error rather than implemented;
`live_smoke` is `#[ignore]`d and unrun (no network/key here — three vendor APIs are therefore verified only
against a mock, and that must be said, not implied); the report's claim that the first gate run failed on
`reachability-check` is a tool-seed coupling, not a product defect (the engine now calls
`resolve_with_observer` for hosted kinds while the xtask seed still watches `resolve_with_transport`).

### The tenth criterion CLOSED (2026-10-10, orchestrator)
`fresh-gate --worktree` on 111's committed HEAD (`1c97da4`): `quality-gate: ALL PASS` and `fresh-gate: PASS`
(exit 0), 82G free. It ran **without** the coverage build — the export's default skip — which is the F7 fix
from spec 110 doing its job: the export is now a hermetic-tree check that fits in the disk, and
`FRESH_GATE_COVERAGE=1` is the explicit way to pay for measuring coverage inside it too. **111: 10/10.**

