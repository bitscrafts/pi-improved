# Spec: 129 — retire the local ollama provider: llama.cpp is the local kind

**Project**: /projects/pi-improved
**Phase**: after the `057` plan (111–128). User directive 2026-10-10, verbatim:
*"ollama wont be part of our setup! only llama.cpp (that will manage all our local models)"* and, clarifying,
*"também temos o ollama instalado no lnx! e deve ser evitado na versão final"*.

## Overview

`pii` currently carries **two** local-model surfaces:

1. an **`ollama:` model kind** — `pii-server/src/lib.rs` (`observed_provider`, `ollama_net_host`,
   `model.starts_with("ollama:")`), honoured by `engine/runs.rs` (the loopback-scoped `NetService` for a live
   local provider), plus test fixtures (`tests/failover_graph.rs`, `node_kinds/agent/tests.rs`,
   `tests/model_tiers.rs` → `model = "ollama:big:70b"`); and
2. a **llama.cpp provider** — `pii-model/src/llamacpp.rs` (`LlamaCppProvider`, behind the `llamacpp` feature,
   compiled by the gate's `clippy (llamacpp)` step).

The directive keeps **one** local kind: llama.cpp. This spec retires the ollama kind — without losing the
*behaviour* that made it valuable (a live local provider may talk to exactly one configured host).


## The concrete dependency (measured 2026-10-10, after the user asked *"algo dependendo do ollama... granite ou laguna?"*)

They were right, and it is **granite**:

- `examples/pii.production.toml:22` — the production example config **requires**
  `model = "ollama:granite4.2:3b"` (the file's own header says a model is REQUIRED to start the daemon), so
  anyone deploying from that example is pointing at the **local ollama** on `127.0.0.1:11434`.
- The same model is **already served by llama.cpp**: the pi/Laguna quadlet
  (`~/.config/containers/systemd/llamacpp-laguna.container`, `localhost/llamacpp-laguna:local`) serves
  `laguna-xs-2.1` **and `granite-4.2-3b`** on `http://10.10.10.217:8082/v1`, and `/root/.pi/agent/models.json`
  declares exactly that under its `llamacpp` provider. amem's own controlled lab measured why:
  *llama.cpp 12.11 tok/s vs ollama 5.61 tok/s — 2.16× faster for the same model* (key
  `hermes-lnx-optimization/validation/llamacpp-vs-ollama`), which is where the "always serve the local GGUF
  via llama.cpp, never via ollama" rule comes from.

So the retirement target is explicit: `ollama:granite4.2:3b` in the production example becomes the llama.cpp
lane (`llamacpp:granite-4.2-3b` against the configured `10.10.10.217:8082` host), and the example's comment must
stop teaching an ollama model spec.

## Requirements

- **R1 — `ollama:` is refused, loudly, with a pointer.** After this spec, a model string beginning `ollama:`
  must fail resolution with a named error whose text names `llamacpp:` as the replacement (e.g.
  `unsupported model kind "ollama:" — this build serves local models through llama.cpp: use llamacpp:<model>`).
  Fail-closed: **no silent fallback** to any other provider, and the refusal is counted/emitted.
- **R2 — llama.cpp becomes the local kind.** `llamacpp:<model>` resolves to the configured llama.cpp host and
  keeps the property the ollama path had: a live local provider is scoped to **exactly that host** (the
  loopback-by-default `NetService` scoping), so a local model cannot reach the network beyond it. Rename
  `ollama_net_host` to reflect the kind it now serves, and keep every call site honest.
- **R3 — config key.** `ollama_host = "http://127.0.0.1:11434"` is retired in favour of the llama.cpp host key
  (default: the loopback llama.cpp endpoint). A config that still carries the **old** key must be **refused
  loudly**, naming the new key — an old config must not silently keep pointing at a service we are removing.
- **R4 — the tests keep testing what they tested.** Update `tests/failover_graph.rs`,
  `node_kinds/agent/tests.rs` and `tests/model_tiers.rs` to the new kind, preserving each test's intent
  (provider failover, agent-node provider plumbing, tier tool gating). A test that only passes because it was
  re-pointed at a different kind, without keeping its original claim, is a regression — say so instead of
  quietly rewriting the assertion.
- **R5 — docs and `001`.** Update the vision/doc lines that mention an ollama model kind; state the rule
  (`local models = llama.cpp only; no local ollama in the final version`) and note that the **lnx host still
  runs `ollama.service` on `127.0.0.1:11434` today** and that nothing may depend on it — disabling the host
  service is a deployment action for the user, not something an agent does.
- **R6 — no new dependency, no feature drift.** `llamacpp` keeps its current feature semantics (already
  compiled by the gate), and no crate gains a dependency for this.
- **R7 — observability stays honest.** Check `pii-observe/src/names.rs` for any name that says ollama and
  rename it (with `ALL` updated); a stale name that describes a retired path is a defect in this repo's own
  terms.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `an_ollama_model_is_refused_pointing_at_llamacpp` | `PII_MODEL=ollama:llama3.2:3b` | refusal; the message names `llamacpp:`; no provider is built |
| `a_llamacpp_model_resolves_to_the_configured_host` | `[llama] host = "http://127.0.0.1:8088"`, `PII_MODEL=llamacpp:q8` | provider built; `NetService` scope = that host only |
| `a_config_with_the_retired_ollama_host_key_is_refused` | config containing `ollama_host = "…"` | loud refusal naming the new key |
| `the_failover_graph_still_fails_over` | updated fixture | unchanged claim: the run fails over to the second provider |
| `tier_tool_gating_still_holds` | updated fixture | unchanged claim: a tier without a tool cannot call it |

## Exit Criteria

- [ ] `cargo test -p pii-server --test failover_graph` — rc 0, and the test still asserts a failover
- [ ] `cargo test -p pii-server --test model_tiers` — rc 0
- [ ] `cargo test -p pii-server node_kinds::agent` — rc 0
- [ ] `cargo test -p pii-model` — rc 0 (llamacpp provider untouched/build-clean)
- [ ] `grep -rn 'ollama' crates/ examples/ graphs/` shows **only**: the refusal text, the retirement test, and a comment explaining the removal — no live `ollama:` path, no `ollama_host` config
- [ ] `cargo run -q -p xtask -- deps-check` and `net-check` and `doc-drift-check` — rc 0 each
- [ ] `bash quality-gate.sh` — rc 0 (it compiles `--features llamacpp`)
- [ ] `PII_MODEL=ollama:x ./target/debug/pii run start …` pasted, showing the refusal text (no silent fallback)
- [ ] `001` and the docs updates pasted

## Guardrails

- Do **not** remove the loopback scoping — it is the security property that made the local kind safe; transfer
  it to llama.cpp rather than deleting it.
- Do **not** touch the host's `ollama.service` (a deployment action, the user's call) and do not "fix" a
  reachability failure by pointing at some other endpoint: the llama.cpp server is not reachable from this
  container, so the tests must stay offline with a mock transport and say so.
- Do **not** widen any dependency or allowlist entry for this; if something seems to need it, stop and report.
- Do **not** edit `specs/`, do **not** commit — leave the tree for the orchestrator.
