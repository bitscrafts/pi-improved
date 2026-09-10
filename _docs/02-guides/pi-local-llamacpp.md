# Calling pi with local llama.cpp models (granite / laguna)

**Date**: 2026-09-10 · **Status**: VERIFIED WORKING

## The setup

Both local models are served by **one llama.cpp router server** on port 8082
(`--models-preset /projects/_tmp/llama-models.ini --models-max 2 --jinja`).
The router exposes an OpenAI-compatible `/v1/chat/completions` and switches
models via the `model` field.

pi talks to it through the **built-in `llamacpp` provider** (NOT a custom
provider name). The provider's `baseUrl` is overridden in
`/root/.pi/agent/models.json` to point at the router:

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

`/root/.pi/agent/settings.json` `enabled_models` includes `"llamacpp/*"`.

## The invocation (CRITICAL flag order)

`--provider` and `--model` MUST come **BEFORE** `-p`. If they come after `-p`,
pi ignores them and falls back to the default provider (anthropic → 429).

```bash
# One-shot prompt
pi --provider llamacpp --model granite-4.2-3b -p "Reply with exactly: GRANITE_OK"
pi --provider llamacpp --model laguna-xs-2.1  -p "Reply with exactly: LAGUNA_OK"

# Agentic (tool-using) run — implement a spec
# NOTE: local models are slow + verbose; raise the tool-iteration cap well above
# the default 50 (they need more iterations to finish). Use 120+.
pi --provider llamacpp --model granite-4.2-3b \
   -p "$(cat _tmp/brief-36-granite-tool-calls.md)" \
   --tools read,write,edit,bash,grep,find,ls \
   --max-tool-iterations 120
```

## Model ids

- `granite-4.2-3b` — IBM Granite 4.2 3B (Q4_K_M, 2.2GB)
- `laguna-xs-2.1` — Laguna XS 2.1 (Q4_K, 20GB)

Both are reasoning models; the router preset sets `reasoning = off` for both so
they emit content (not an empty `reasoning_content` trace).

## Verify

```bash
pi --list-models llamacpp   # shows both models
curl -s http://localhost:8082/v1/models | jq -r '.data[].id'  # router alive
```

## Pitfalls

- **Flag order**: `--provider`/`--model` before `-p`, or pi falls back to the
  default provider.
- **Use the built-in `llamacpp` provider**, not a custom name like
  `laguna-local`/`granite-local` — pi's static provider table only knows
  `llamacpp` (and `ollama`, `mistralrs`) as keyless local providers.
- **`pi` is at `/root/.local/bin/pi`** — not always on PATH in background shells.
- The router must be running (port 8082). If it's down, pi's request fails.
