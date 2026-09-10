# Making llama.cpp + granite work on pi — the full process

**Date**: 2026-09-10 · **Status**: VERIFIED WORKING

This documents the complete, end-to-end process of getting local llama.cpp
models (IBM Granite 4.2, Laguna XS) working with pi — both as an external
router server AND as an in-process provider inside Pii. It is the "how we got
here" companion to `pi-local-llamacpp.md` (the quick reference).

## 1. The models

| Model | File | Size | Notes |
|-------|------|------|-------|
| Granite 4.2 3B | `/projects/_tmp/granite-4.2-3b-Q4_K_M.gguf` | 2.2GB | IBM, Apache-2.0, non-thinking via `reasoning=off` |
| Laguna XS 2.1 | `/models/laguna-xs-2.1.gguf` | 20GB | Q4_K, reasoning model |

Both are **reasoning models** — they emit `reasoning_content` and leave
`content` empty unless `reasoning = off` is set. This is the same trap as
glm-5.3/deepseek-v4 (memory insight). Without `reasoning=off`, a request burns
its token budget on a thinking trace and returns empty content.

## 2. Serve both on ONE llama.cpp router (port 8082)

Instead of two separate `llama-server` processes, use llama.cpp **router mode**:
one server, both models, switch via the `model` field.

**`/projects/_tmp/llama-models.ini`**:
```ini
version = 1

[*]
mmap = 1
parallel = 1
jinja = 1

[laguna-xs-2.1]
model = /models/laguna-xs-2.1.gguf
c = 32768
temp = 0.6
top-p = 0.95
min-p = 0
top-k = 40
reasoning = off

[granite-4.2-3b]
model = /projects/_tmp/granite-4.2-3b-Q4_K_M.gguf
c = 32768
temp = 0.6
top-p = 0.95
min-p = 0
top-k = 40
reasoning = off
```

**Start it**:
```bash
llama-server --models-preset /projects/_tmp/llama-models.ini \
  --host 0.0.0.0 --port 8082 --threads 16 --models-max 2 --jinja
```

The router spawns a child `llama-server` per model on random ports and proxies
requests. Verify:
```bash
curl -s http://localhost:8082/v1/models | jq -r '.data[].id'
# granite-4.2-3b
# laguna-xs-2.1
```

## 3. Configure pi to use the router (the built-in `llamacpp` provider)

pi has a **built-in `llamacpp` provider** (keyless, local, OpenAI-compatible).
Do NOT invent custom provider names like `laguna-local`/`granite-local` — pi's
static provider table only knows `llamacpp`, `ollama`, `mistralrs` as keyless
local providers. Override its `baseUrl` to the router port.

**`/root/.pi/agent/models.json`**:
```json
"llamacpp": {
  "api": "openai-completions",
  "baseUrl": "http://localhost:8082/v1",
  "apiKey": "not-needed",
  "authHeader": false,
  "models": [
    { "id": "laguna-xs-2.1",   "contextWindow": 32768, "maxTokens": 8192 },
    { "id": "granite-4.2-3b",  "contextWindow": 32768, "maxTokens": 8192 }
  ]
}
```

**`/root/.pi/agent/settings.json`** — `enabled_models` must include `"llamacpp/*"`.

## 4. The invocation — CRITICAL flag order

`--provider` and `--model` MUST come **BEFORE** `-p`. If they come after `-p`,
pi ignores them and falls back to the default provider (anthropic → 429).

```bash
# WRONG — falls back to anthropic:
pi -p "hi" --provider llamacpp --model granite-4.2-3b

# RIGHT:
pi --provider llamacpp --model granite-4.2-3b -p "Reply with exactly: GRANITE_OK"
pi --provider llamacpp --model laguna-xs-2.1  -p "Reply with exactly: LAGUNA_OK"
```

This was the hardest bug to find: the flag order silently dropped the provider
override, and the error surfaced as an anthropic 429 (the default provider),
which looked like a quota problem, not a routing problem.

## 5. Verify pi reaches the models

```bash
pi --list-models llamacpp   # shows both models
pi --provider llamacpp --model granite-4.2-3b -p "Reply with exactly: GRANITE_OK"
# GRANITE_OK
```

## 6. ds4-bench on the router (model-level capability)

Both models pass all 4 ds4-bench autonomy benchmarks via the router:

| Benchmark | Granite-4.2-3b | Laguna-XS-2.1 |
|-----------|----------------|---------------|
| B1 tool-calling (full 18-tool) | PASS (1 call, 33.8s) | PASS (1 call, 122.7s) |
| B1 tool-calling (minimal 7-tool) | PASS (1 call, 8.4s) | PASS (1 call, 8.2s) |
| B2 speed | 12.88 tok/s | 13.32 tok/s |
| B3 2-step autonomy | PASS (13.1s) | PASS (7.3s) |
| B4 3-step autonomy | PASS (11.9s) | PASS (206s) |

This confirms the memory insight `benchmark-corrected`: local models DO have
multi-step autonomy; the bottleneck is speed (~13 tok/s CPU), not autonomy.

## 7. In-process provider (spec 35) — embed granite inside Pii

The router is an external server. Spec 35 added the **in-process** path: link
llama.cpp directly into the Pii binary via `llama-cpp-2`, so granite runs inside
Pii with no external server and no `NET` capability.

- `crates/pii-model/src/llamacpp.rs` — `LlamaCppProvider` implementing the sync
  `ModelProvider` trait (tokenize→batch→decode→sample loop).
- `granite::build_prompt` — hand-built `<|start_of_role|>...<|end_of_role|>`
  wire format (NOT `apply_chat_template`), so non-thinking is explicit. Ends
  with the `<|start_of_role|>assistant<|end_of_role|>` generation trigger.
- `provider_from_spec("llamacpp:<path>")` — the spec arm.
- `[core] model = "llamacpp:/path.gguf"` — the config entry.
- Real-model acceptance test `real_granite_chat` (`#[ignore]`, manual): loads
  the 2.2GB GGUF and answers `GRANITE_OK`.

**Build requirements**: `cmake` + `clang` (llama.cpp's CMake emits Clang-only
warning flags that GCC rejects). Force clang via `.cargo/config.toml`:
```toml
[env]
CC = "clang"
CXX = "clang++"
[alias]
xtask = "run -p xtask --"
```

## 8. Next: in-process tool calls (spec 36)

The in-process provider is text-only (`tool_calls: Vec::new()`). Spec 36 adds
structured tool calls via `json_schema_to_grammar` + `LlamaSampler::grammar`
(constrained generation), so ds4-bench's B1/B3/B4 become meaningful against
granite running inside Pii. This is currently being implemented by granite
itself (the workhorse).

## Pitfalls recap

1. **`reasoning = off`** for both models, or they emit empty content.
2. **Use the built-in `llamacpp` provider**, not custom names.
3. **`--provider`/`--model` before `-p`**, or pi falls back to the default.
4. **`pi` is at `/root/.local/bin/pi`** — not always on PATH in background shells.
5. **`cmake` + `clang`** required for the in-process build; force clang via
   `.cargo/config.toml`.
6. The router must be running (port 8082) for the external path.
