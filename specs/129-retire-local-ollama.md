# Spec: 129 — retire the local ollama provider; llama.cpp (the server) is the local kind

**Project**: /projects/pi-improved
**Phase**: after the `057` plan (111–128). User directive 2026-10-10, verbatim:
*"ollama wont be part of our setup! only llama.cpp (that will manage all our local models)"*, *"também temos o
ollama instalado no lnx! e deve ser evitado na versão final"*, and on the granite model specifically:
*"exatamente e esse modelo deve correr no llama.cpp"*.

## Overview

Two things are called "llama.cpp" here, and this spec must not conflate them:

1. **`pii`'s `llamacpp:<path>`** — `pii-model/src/llamacpp.rs` loads a **GGUF file in-process** (behind the
   `llamacpp` feature). It is not a network provider: it has no host, and `egress_host` returns `None` for it.
   **It stays exactly as it is.**
2. **The llama.cpp *server*** — an HTTP, OpenAI-compatible endpoint that already serves our local models:
   the pi/Laguna quadlet on **`http://10.10.10.217:8082/v1`** serves `laguna-xs-2.1` **and `granite-4.2-3b`**
   (declared in `/root/.pi/agent/models.json` under the `llamacpp` provider), and laya's backbone is served the
   same way on `:8083`. This is what "llama.cpp manages all our local models" means, and `pii` has **no client
   for it today** — which is why the production example still points at the local ollama
   (`examples/pii.production.toml:22` → `model = "ollama:granite4.2:3b"`).

Measured context that motivates the whole directive (amem `hermes-lnx-optimization/validation/llamacpp-vs-ollama`):
**llama.cpp 12.11 tok/s vs ollama 5.61 tok/s for the same model (2.16×)**, on the same hardware — "always
serve the local GGUF via llama.cpp, never via ollama".

The previous attempt at this spec was **stopped at the premise check by the implementer**, correctly: the spec
had assumed `llamacpp:` was already an HTTP provider. It is not. This revision states the design explicitly.

## Requirements

- **R1 — `ollama:` is refused, loudly, with a pointer.** A model string beginning `ollama:` fails resolution
  with a named error whose text names `llamacpp-server:` (this spec's new kind) — fail-closed, **no silent
  fallback**, and the refusal is counted/emitted.
- **R2 — a new HTTP kind, `llamacpp-server:<model>`, is the local kind.**
  - It is implemented by **reusing the existing OpenAI-compatible provider** (llama.cpp's server speaks that
    API; the provider already reads `OPENAI_BASE_URL`/`OPENAI_API_KEY`). **No new dependency.**
  - Its host comes from the config: `[llama] host = "http://…/v1"`, **default loopback**
    (`http://127.0.0.1:8080/v1`). Per-model, the model name is appended as the OpenAI-compatible provider
    already does.
  - It keeps the property the `ollama:` path had: a live local provider is scoped, through the same
    `NetService`/egress-host mechanism, to **exactly that host** — a local model cannot reach the network
    beyond it. Rename the plumbing that still says `ollama` to match the kind it now serves.
  - `llamacpp:<path>` (in-process GGUF) is **untouched**: same feature, same semantics, same tests. The two
    names must be distinguishable in the error you get for a wrong one.
- **R3 — config keys.** The retired `ollama_host` key is gone; a config that still carries it is **refused
  loudly, naming `[llama] host`**. The new key's default is loopback.
- **R4 — the tests keep testing what they tested.** `tests/failover_graph.rs`, `node_kinds/agent/tests.rs`,
  `tests/model_tiers.rs` (its `ollama:big:70b` fixture) move to the new kind, preserving each test's original
  claim (failover, agent-node provider plumbing, tier tool gating). A test that only passes because it was
  re-pointed without keeping its claim is a regression — say so instead of quietly rewriting the assertion.
- **R5 — the production example stops teaching an ollama model.** `examples/pii.production.toml` uses
  `llamacpp-server:granite-4.2-3b` with `[llama] host = "http://10.10.10.217:8082/v1"`, and its comment names
  the quadlet that serves it (`granite-4.2-3b` + `laguna-xs-2.1`) so the next reader knows where the model
  lives. Docs and `001` updated; state that the host's `ollama.service` (`127.0.0.1:11434`, unreachable from
  the container — measured `000`) must not be depended on and is a user-side deployment step.
- **R6 — footprint.** No new dependency, no widened allowlist, no change to the `llamacpp` feature or its
  gate step. If something seems to need more, stop and report.
- **R7 — observability stays honest.** Rename/retire any name in `pii-observe/src/names.rs` that describes the
  retired path, updating `ALL`; a stale name that describes a removed path is a defect in this repo's terms.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `an_ollama_model_is_refused_pointing_at_llamacpp_server` | `PII_MODEL=ollama:granite4.2:3b` | refusal; the message names `llamacpp-server:`; no provider built |
| `a_llamacpp_server_model_resolves_to_the_configured_host` | `[llama] host = "http://10.10.10.217:8082/v1"`, `PII_MODEL=llamacpp-server:granite-4.2-3b` | provider built; egress scope = that host only |
| `a_config_with_the_retired_ollama_host_key_is_refused` | `ollama_host = "…"` in the config | loud refusal naming `[llama] host` |
| `the_in_process_gguf_kind_is_unchanged` | `llamacpp:/path/to/model.gguf` | still the in-process loader; no host, no NetService |
| `the_failover_graph_still_fails_over` | updated fixture | unchanged claim |
| `tier_tool_gating_still_holds` | updated fixture | unchanged claim |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib` — rc 0, with the four new tests above present and green
- [ ] `cargo test -p pii-server --test failover_graph` and `--test model_tiers` — rc 0, claims unchanged
- [ ] `cargo test -p pii-model` and `cargo test -p pii-model --features llamacpp` — rc 0
- [ ] `grep -rn 'ollama' crates/ examples/ graphs/` shows only: the refusal text, the retirement test, and a comment explaining the removal — no live `ollama:` path, no `ollama_host` key
- [ ] `PII_MODEL=ollama:x ./target/debug/pii run start <graph>` pasted: the refusal text, no fallback
- [ ] `cargo run -q -p xtask -- deps-check` / `net-check` / `doc-drift-check` — rc 0 each
- [ ] `bash quality-gate.sh` — rc 0
- [ ] the production example + docs + `001` diffs pasted

## Guardrails

- **Do not conflate the two llama.cpp things.** `llamacpp:<path>` stays in-process; the server kind is new
  and HTTP. If you find yourself changing the GGUF loader, stop — that is not this spec.
- **Do not delete the loopback scoping** — transfer it to the new kind; it is the security property that made
  a local provider safe.
- **Do not touch the host's `ollama.service`**, and do not "fix" the unreachable llama.cpp host by inventing a
  different endpoint: this container cannot reach `10.10.10.217:8082`, so tests stay offline with a mock
  transport and say so.
- **Do not widen anything** (dependencies, allowlists, budgets). If it seems necessary, stop and report.
- Do **not** edit `specs/`, do **not** commit — leave the tree for the orchestrator.
