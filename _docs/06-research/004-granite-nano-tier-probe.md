# Granite 4.2 nano/small-tier probe + flexible model coupling

**Date**: 2026-09-02 · **Author**: orchestrator (probe) + opus (recommendation) · **Status**: DECISION-IN-PROGRESS

## Context
The pass-3 architecture (`003-pii-architecture-plan-pass3.md`) defines:
- **Tier 0 (nano)** = route/classify/extract, one call, no reasoning, escalates; 10–100ms, $0.
  Default backend is **B1** (an `Arc<dyn Embedder>` + fitted head, pure Rust, no chat model) — pass-3 §3.1.
  Do **not** use a chat LLM at tier 0 (occupies the single Ollama slot, violates latency).
- **Tier 1 (small)** = local ~9B via Ollama/llama.cpp, ~1s: routing, promoted runs, tool-arg repair, summarisation, cheap planning — pass-3 §2 row `:175`.

## opus recommendation (2026-09-02, grounded in pass-3)
- Standardize tier 1 on **granite4.2:8b** (primary, Apache-2.0, 5.3GB/128K) + **granite4.2:3b** (same-family fast lane).
- **ornith-1.5:9b** = fallback only; its **license is UNVERIFIED** (ollama page states none) and its thinking-control is unknown.
- **No chat model at tier 0** — B1 embedder stays.
- Non-negotiable **adoption gate**: one-shot probe `max_tokens≈6000` must show content>0 AND reasoning==0, with the exact wire field recorded.

## Official IBM Granite 4.2 doc (authoritative, https://www.ibm.com/granite/docs/models/granite4-2)
- Dense reasoning family, 3B/8B/30B, **Apache 2.0** (crypto-signed, ISO certified), 128K ctx (512K on 30B).
- **Thinking modes are chat-template parameters**:
  | Mode | Template Params |
  |---|---|
  | Thinking (default) | `enable_thinking=True` — full CoT in `thinking...response` |
  | **Non-thinking** | `enable_thinking=False` — **direct answer, no CoT** |
  | Low-effort | `enable_thinking=True, low_effort=True` — brief reasoning |
- Generation params: `temperature=1.0, top_p=0.95` across all tasks/backends.
- **IBM optimizes for vLLM** (v0.20+): `vllm serve ... --reasoning-parser granite_thinking_parser --tool-call-parser qwen3_coder --enable-auto-tool-choice`, exposes OpenAI-compatible API. Also integrates with agentic harnesses (OpenCode, Pi, OpenHands) out of the box.
- This confirms llama.cpp `--reasoning off` == `enable_thinking=False` (Non-thinking mode).

## Probe (verified on THIS no-GPU box, via llama.cpp)
Model: `granite-4.2-3b-Q4_K_M.gguf` (2.24GB) from `ibm-granite/granite-4.2-3b-GGUF` on HF.
Server: `llama-server -m ... -c 4096 --reasoning off` (llama.cpp server build `a1f96d4`).

| Config | Result |
|---|---|
| Default (thinking ON) | `content: ''`, `reasoning_content` burns budget, `finish: length` — **same empty-content trap as glm-5.3** |
| `--reasoning off` (== Non-thinking) | `content: '42'`, `reasoning_content: NONE`, `finish: stop`, ~28 tok/s; JSON output clean |

**Conclusion**: granite-4.2 is CPU-viable for tier 1 and passes the thinking-off gate, **but ONLY when served with `--reasoning off`** (llama.cpp) or `enable_thinking=False` (transformers/vLLM). Notes:
- The modern llama-server flag is `--reasoning off` (`-rea`). The older `--chat-template-kwargs '{"enable_thinking":false}'` is now **deprecated** (server warns "Use --reasoning on/off instead"). Body-level `enable_thinking:false` / `reasoning_effort` are **not honored** by this llama.cpp server build.
- On Ollama, reasoning is NOT controllable at all (Ollama replaces the model's own Jinja template that carries the toggle) — so granite tier-1 must run via llama.cpp or vLLM, not Ollama.
- CPU speed ~14–28 tok/s on 32 cores / no GPU. Fine for tier-1 ~1s latency budget on short prompts.

## Flexible model coupling (user directive, 2026-09-02)
> "when attaching to pii, make the coupling flexible, allowing the owner to attach any model (if possible and as future work)"

**Design intent**: model backends MUST be pluggable — a driver trait / factory the owner configures (`model[provider]`-style, mirroring how the worker chain already picks providers), NEVER hardwired to one vendor. Concrete consequences:
- A `ModelProvider`-style trait (sync `chat(&Ctx, &ChatRequest) -> ChatResponse`) already anticipated in the Phase-2 plan (`pii-model`); keep it the single seam so an owner can attach granite/llama.cpp/vLLM, ornith/GGUF, ollama-cloud, or any OpenAI-compatible endpoint. IBM's own doc confirms granite's OpenAI-compatible API "integrates with ... Pi" — so the seam is a standard one.
- Keep the nano ROLE (route/classify/extract) decoupled from any specific model — a `NanoRouter` impl can be embedder-backed (B1, default) OR an LLM-backed driver attached later (B2′). Tier 0 stays B1 by default.
- This is "attach any model" as future work; record here so later phases keep the seam open.

## Actionable next steps (not yet done)
- [ ] Save this as the canonical decision doc; link from `_docs/INDEX.md` / `TOC.md`.
- [ ] In the Phase-2 `pii-model` spec, keep `ModelProvider` the single pluggable seam (per above).
- [ ] Keep `granite-4.2-3b-Q4_K_M.gguf` at `/projects/_tmp/` as the verified tier-1 candidate; note in `AGENTS.md`/models config that tier-1 must run via llama.cpp `--reasoning off`, not Ollama.
- [ ] `ornith-1.5` license + thinking-control: verify before any adoption.
