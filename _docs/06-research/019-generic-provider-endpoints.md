# Generic OpenAI + Anthropic endpoint support for pii-model

**Date**: 2026-09-10 · **Author**: orchestrator (research) ·
**Status**: RESEARCH — no code changed. Recommends a concrete integration
plan for a future spec; do not implement from this doc without turning it
into a spec first.

## 1. Why this exists

Pii's model seam (`crates/pii-model`) currently has exactly one live HTTP
provider: `OllamaProvider`, which happens to already speak the
OpenAI-compatible `/api/chat` dialect. Almost every other LLM vendor exposes
one of two API shapes:

- **OpenAI Chat Completions** (`POST /v1/chat/completions`) — used natively
  by OpenAI, and copied verbatim by Groq, Together, DeepSeek, Mistral,
  Cerebras, xAI, OpenRouter, vLLM, llama.cpp, and Ollama itself.
- **Anthropic Messages** (`POST /v1/messages`) — used natively by Anthropic,
  and copied by some relays/proxies.

Adding two generic providers — `OpenAiProvider` and `AnthropicProvider` —
each with a configurable `base_url`, lets Pii talk to nearly any hosted or
self-hosted LLM without a provider-specific integration per vendor. The
reference implementation is `_repos/pi_agent_rust` (`src/providers/openai.rs`,
`src/providers/anthropic.rs`, `src/provider.rs`), a mature, async, streaming
Rust client for both shapes plus half a dozen dialect variations (DeepSeek
thinking, OpenRouter routing, SAP AI Core auth, Kimi OAuth, Azure). Pii should
steal the *wire shapes*, not the *dialect surface area* — see §4.

## 2. API-shape comparison

### 2.1 Request body

| | OpenAI Chat Completions | Anthropic Messages |
|---|---|---|
| Endpoint | `POST {base}/chat/completions` (default `https://api.openai.com/v1/chat/completions`) | `POST {base}` (default `https://api.anthropic.com/v1/messages`) — note pi_agent_rust's `ANTHROPIC_API_URL` already includes the full path, unlike OpenAI's which is joined from a base |
| Auth header | `Authorization: Bearer <key>` | `x-api-key: <key>` (or `Authorization: Bearer` for OAuth-style tokens — irrelevant to Pii) |
| Version header | none | `anthropic-version: 2023-06-01` (**required on every request**) |
| System prompt | A `{"role":"system","content":...}` message prepended to `messages` | A **top-level** `system` field (string or array of `{"type":"text","text":...}` blocks) — never a message in the `messages` array |
| Messages roles | `system`, `user`, `assistant`, `tool` | `user`, `assistant` only — tool results are `user`-role messages with `tool_result` content blocks |
| Tool call encoding (request→model, i.e. echoing history back) | Assistant message carries `tool_calls: [{id, type:"function", function:{name, arguments: <JSON string>}}]` | Assistant message content includes a `{"type":"tool_use", id, name, input: <JSON value>}` block (input is a JSON *value*, not a string) |
| Tool result encoding | A separate message: `{"role":"tool", "tool_call_id":..., "content": "..."}` | A `user`-role message with content `[{"type":"tool_result", tool_use_id, content:[...text/image blocks...], is_error?}]` |
| Tool schema | `{"type":"function","function":{name,description,parameters:<JSONSchema>}}` | `{"name","description","input_schema":<JSONSchema>}` — flat, no `type`/`function` wrapper |
| Max tokens | `max_tokens` (or `max_completion_tokens` on o1-series/reasoning models) | `max_tokens` — **required** field, no server default |
| Temperature | `temperature` (0–2) | `temperature` (0–1); omitted entirely when "thinking" is enabled (Anthropic rejects sampling params with thinking on) |
| Streaming flag | `"stream": true` + optional `"stream_options": {"include_usage": true}` | `"stream": true` (usage always included in the terminal `message_delta`) |
| Multi-turn history | Flat list of messages, system message(s) allowed anywhere but conventionally first | `system` is out-of-band; `messages` must strictly alternate `user`/`assistant` in Anthropic's model (tool results count as `user` turns) |

### 2.2 Streaming (SSE) format

Both use `Content-Type: text/event-stream` with `data: <json>\n\n` frames,
terminated differently:

**OpenAI**: a flat sequence of `chat.completion.chunk` objects, each with
`choices[0].delta` carrying an incremental field (`content`,
`reasoning_content` for DeepSeek, or `tool_calls[i].function.arguments` as a
growing string fragment matched by array `index`). No block lifecycle — text
just accumulates by concatenation. Termination is the literal
sentinel `data: [DONE]`, not a typed event. Tool-call arguments arrive as
raw JSON-string fragments with **no matching close event**; the consumer
must buffer until `finish_reason` arrives, then `serde_json::from_str` the
concatenated string.

**Anthropic**: a typed event *state machine* — `message_start` →
(`content_block_start` → `content_block_delta`* → `content_block_stop`)* →
`message_delta` → `message_stop`, plus periodic `ping` and an `error` event.
Each content block is explicitly typed (`text`, `tool_use`, `thinking`) via
`content_block_start.content_block.type`; deltas are typed too
(`text_delta`, `input_json_delta` for tool-call argument fragments,
`thinking_delta`). Tool arguments stream as `partial_json` fragments under
`input_json_delta`, closed cleanly by `content_block_stop`. Usage is only
in `message_start.message.usage` (input tokens) and
`message_delta.usage` (output tokens) — never per-chunk like OpenAI.

**Where they're isomorphic**: both are pure line-oriented SSE (`data: ` +
JSON per event), both encode incremental tool-call argument fragments that
must be concatenated and parsed once complete, and both report cumulative
usage near the end of the stream. A single "SSE line-splitter" helper
(buffer partial lines across TCP reads, split on `\n\n`/`\ndata: `) can
serve both providers — see §3.3.

**Where they diverge and it matters for Pii's `ChatResponse`**: OpenAI has
no explicit block lifecycle (map to Pii's flat `text` accumulator directly);
Anthropic requires tracking `content_block_start` → index → type to know
whether a `content_block_delta` is text or a tool argument fragment — this
is strictly more bookkeeping than Ollama's NDJSON, but no more than the
existing `OllamaProvider::drain_lines` cross-chunk buffering already does.

### 2.3 Non-streaming shape (relevant since `ModelProvider::chat` is the
required, non-optional trait method)

Both APIs support `"stream": false` and return the same information as a
single JSON object instead of an event sequence:

- OpenAI: `{"choices":[{"message":{"role":"assistant","content":"...",
  "tool_calls":[...]}, "finish_reason":"..."}], "usage":{"prompt_tokens",
  "completion_tokens","total_tokens"}}`.
- Anthropic: `{"content":[{"type":"text","text":"..."}, {"type":"tool_use",
  "id","name","input"}], "stop_reason":"...", "usage":{"input_tokens",
  "output_tokens"}}`.

pi_agent_rust's providers are streaming-only (`"stream": true` is
hardcoded in `build_request`); Pii's `ModelProvider::chat` needs the
non-streaming shape too (mirroring how `OllamaProvider::chat` uses
`"stream": false` while `chat_streaming` uses `"stream": true`). Both
vendors support both modes with the same field set (streaming just chunks
the same content), so implementing `chat` first and treating `chat_streaming`
as an SSE decode of the same request body (with `stream: true` flipped) is
the natural order — same shape pi_agent_rust took contractually if you
squint (it decodes the streamed events back into one accumulated message,
which is exactly what `ChatResponse` already is in Pii).

## 3. Generic provider design for `pii-model`

### 3.1 Trait shape recap (already read, `crates/pii-model/src/lib.rs`)

`ModelProvider` is **sync**: `fn chat(&self, ctx: &Ctx, req: &ChatRequest) ->
Result<ChatResponse, PiiError>` plus a default-implemented
`chat_streaming(&self, ctx, req, on_delta: &mut dyn FnMut(&str)) ->
Result<ChatResponse, PiiError>`. `ChatRequest`/`ChatMessage`/`ToolCall`/
`ToolSpec`/`ChatResponse` are provider-agnostic (role enum: System/User/
Assistant/Tool; `ChatMessage.tool_calls: Vec<ToolCall>` on assistant turns;
`ChatMessage.tool_call_id: Option<String>` on tool-result turns). This shape
already matches OpenAI's message model closely (it's isomorphic to what
`OllamaProvider::message_value` builds) and is straightforward to project
into Anthropic's shape (System→top-level `system`, Tool→`user`+
`tool_result` block, Assistant.tool_calls→`tool_use` blocks).

### 3.2 No async bridge needed — `pii-net`'s `HttpTransport` is already sync

The brief raises "blocking thread + channel" as one option for bridging
pi_agent_rust's async `Provider::stream` to Pii's sync `ModelProvider`. This
is **unnecessary**: `pii_core::HttpTransport` (`crates/pii-core/src/net.rs`)
is defined as:

```rust
pub trait HttpTransport: Send + Sync {
    fn request(&self, ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError>;
    fn request_streaming(
        &self, ctx: &Ctx, req: &HttpRequest,
        on_chunk: &mut dyn FnMut(&[u8]),
    ) -> Result<HttpResponse, PiiError> { /* default: one big chunk */ }
}
```

`request_streaming` already delivers raw body bytes to a synchronous
callback as they arrive off the socket — this is exactly what
`OllamaProvider::chat_streaming` uses today for NDJSON (`drain_lines` buffers
partial lines across chunk boundaries). SSE for OpenAI/Anthropic is the same
shape problem (line-oriented, partial reads split mid-frame) with a
different frame delimiter (`data: ...\n\n` instead of bare `\n`-delimited
JSON). **No thread, no channel, no async runtime dependency is needed in
`pii-model`** — write an SSE line-buffer analogous to `drain_lines`, driven
by the same `request_streaming` chunk callback. This also means
`pii-model` need not depend on `tokio`/`futures`/`async-trait` at all,
keeping the "zero socket code, sync everywhere" invariant pi_agent_rust
does not have to honor.

HTTPS is already supported end-to-end: `pii-net`'s `http_codec.rs` parses
`https://` URLs (spec 17), and `dial.rs`/`service.rs` wrap the TCP stream in
rustls when `Scheme::Https` is selected. `api.openai.com` and
`api.anthropic.com` are reachable through the existing transport with no
`pii-net` changes — confirming the doc comment in `ollama.rs` ("Plaintext
`http://` only, TLS is Phase 3b") is stale for the workspace as a whole,
even though `OllamaProvider` itself has never been updated to build an
`https://` URL (irrelevant here since Ollama is always local).

### 3.3 New files

```
crates/pii-model/src/openai.rs      — OpenAiProvider (Chat Completions shape)
crates/pii-model/src/anthropic.rs   — AnthropicProvider (Messages shape)
crates/pii-model/src/sse.rs         — shared SSE line-buffer (used by both)
```

`sse.rs` factors out a `SseLineBuffer` with a `push(&mut self, bytes: &[u8],
on_event: &mut dyn FnMut(&str))` method that:
1. Appends `bytes` to an internal `String` buffer.
2. Splits on `\n\n` (SSE frame boundary), keeping any trailing partial frame
   buffered (mirrors `OllamaProvider::drain_lines`'s residual-buffer pattern,
   R3-style cross-chunk safety).
3. For each complete frame, strips the `data: ` prefix per line (SSE frames
   can have multiple `data:` lines to concatenate) and hands the JSON string
   to `on_event`.
4. Ignores non-`data:` lines (`event:`, `id:`, comments starting with `:`)
   and blank keep-alive frames.

Both `OpenAiProvider::chat_streaming` and `AnthropicProvider::chat_streaming`
drive this buffer from `HttpTransport::request_streaming`'s chunk callback,
exactly as `OllamaProvider::chat_streaming` drives its NDJSON buffer today.
`[DONE]` (OpenAI) and `message_stop` (Anthropic) are provider-specific
terminal markers detected in each provider's `on_event` closure, not in the
shared buffer.

### 3.4 `OpenAiProvider`

```rust
pub struct OpenAiProvider {
    transport: Arc<dyn HttpTransport>,
    base_url: String,      // e.g. "https://api.openai.com/v1" or
                            // "https://api.groq.com/openai/v1" — full path
                            // up to but excluding "/chat/completions"
    model: String,
    api_key: Option<String>,
    timeout_ms: u64,
}
```

- `chat_url()` = `format!("{base_url}/chat/completions")` — matches
  pi_agent_rust's `with_base_url` convention where the base is joined, not
  the full URL (unlike its `ANTHROPIC_API_URL` constant, which is the full
  path — keep Pii's two providers internally consistent by having *both*
  take a base and append the fixed suffix, since that's what makes
  "any OpenAI-compatible endpoint" / "any Anthropic-compatible endpoint"
  actually configurable without the caller needing to know the exact path).
- `build_request`: mirror `OllamaProvider::message_value`/`tool_value` almost
  verbatim — Pii's `ChatMessage`/`ToolSpec` already have the right shape.
  New work versus Ollama: encode `Role::System` as an OpenAI `system`
  message (Ollama already does this — no change needed there, OpenAI's
  wire shape for messages is identical to what `message_value` already
  produces). The only real additions versus `OllamaProvider::build_request`:
  `"stream_options": {"include_usage": true}` when streaming, and using
  `Authorization: Bearer` instead of Ollama's optional bearer.
- Non-streaming `chat`: parse `choices[0].message.{content,tool_calls}` +
  `usage.{prompt_tokens,completion_tokens}` — steal the field names from
  pi_agent_rust's `OpenAIUsage`/`OpenAIChoice` structs (lines ~1438-1507 of
  `openai.rs`) but as a plain `#[derive(Deserialize)]` struct, no `Cow`/
  lifetime gymnastics needed since Pii doesn't need zero-copy (Ollama's
  provider doesn't bother either).
- Streaming `chat_streaming`: parse each SSE `data:` line as
  `OpenAIStreamChunk` (steal the struct shape from pi_agent_rust lines
  1438-1481), forward `choices[0].delta.content` through `on_delta`
  incrementally, accumulate `tool_calls` by `index` (steal the
  index-based upsert logic from `process_choice`, lines 1144-1257 of
  `openai.rs` — this is the trickiest part to get right: OpenAI can emit
  sparse/interleaved tool-call indices across chunks). Stop on literal
  `data: [DONE]`.
- **Not stealing**: DeepSeek `reasoning_style`/`thinking` dialect, OpenRouter
  routing overrides, SAP AI Core / Kimi auth branches, `complete_partial_json`
  (partial-JSON-completion for live UI rendering — Pii's `on_delta` is text
  deltas only, tool-call args are delivered whole at the end like Ollama
  already does), `BeforeProviderRequestHook`. All of that is pi_agent_rust's
  CLI-app-specific surface area, not part of the wire protocol.

### 3.5 `AnthropicProvider`

```rust
pub struct AnthropicProvider {
    transport: Arc<dyn HttpTransport>,
    base_url: String,   // full path, e.g. "https://api.anthropic.com/v1/messages"
    model: String,
    api_key: Option<String>,
    timeout_ms: u64,
}
```

- Headers: `x-api-key: <key>`, `anthropic-version: 2023-06-01` (steal the
  constant), `Content-Type: application/json`. Skip the OAuth
  bearer-vs-x-api-key branching, the `anthropic-beta` prompt-cache flags,
  and the 1h-TTL/`is_first_party_anthropic_base_url` logic entirely for a
  first cut — those are cost-optimization features orthogonal to "can Pii
  talk to Anthropic-shaped endpoints at all." (Flag as a natural follow-up:
  once `AnthropicProvider` exists, prompt-cache breakpoints are a pure
  request-body addition gated on `is_first_party_anthropic_base_url`,
  exactly as pi_agent_rust does it — worth a future spec, not this one.)
- `build_request`: project `ChatRequest` into `{model, system, messages,
  max_tokens, temperature, tools, stream}`:
  - Pull all `Role::System` messages' `content` into the top-level `system`
    field (join with `\n\n` if Pii ever allows more than one — today
    callers send at most one system message per the existing Ollama path).
  - `Role::User`/`Role::Assistant` map straight across as
    `{"role": "user"|"assistant", "content": [{"type":"text","text":...}]}`.
  - `Role::Tool` (Pii's tool-result message) maps to a `user`-role message
    with `content: [{"type":"tool_result", "tool_use_id": tool_call_id,
    "content":[{"type":"text","text":...}]}]` — steal the shape from
    `convert_message_to_anthropic`'s `Message::ToolResult` arm
    (`anthropic.rs:1481-1505`), dropping the image-block branch (Pii's
    `ChatMessage.content` is a single `String`, no multi-part content today).
  - Assistant messages carrying `tool_calls` (Pii's `ChatMessage.tool_calls`)
    map each entry to a `{"type":"tool_use", id, name, input: arguments}`
    content block, alongside a `{"type":"text","text":...}` block if
    `content` is non-empty — steal from `convert_content_block_to_anthropic`.
  - Tools: `{"name","description","input_schema": schema}` — steal
    `convert_tool_to_anthropic` verbatim (it's a 5-line function).
  - `max_tokens` is **required** by the API — default it (e.g. 4096) if
    `ChatRequest.max_tokens == 0`, matching `ChatRequest.max_tokens: u32`'s
    existing "0 means unset" convention used for `num_ctx` in Ollama.
- Non-streaming `chat`: parse `{content: [...], stop_reason, usage:
  {input_tokens, output_tokens}}` — walk `content` blocks, concatenating
  `text` blocks into `ChatResponse.text` and collecting `tool_use` blocks
  into `ChatResponse.tool_calls` (id/name/input map directly to Pii's
  `ToolCall{id,name,arguments}` — no translation needed).
- Streaming `chat_streaming`: drive `SseLineBuffer`, `serde_json::from_str`
  each event with `#[serde(tag = "type", rename_all = "snake_case")]` into
  an enum mirroring pi_agent_rust's `AnthropicStreamEvent` (steal verbatim,
  `anthropic.rs:1336-1363`, minus the `Ping` no-op which can just be
  ignored). State machine to track: on `content_block_start` record the
  block's type by index; on `content_block_delta` with `text_delta` forward
  through `on_delta` and accumulate into `ChatResponse.text`; with
  `input_json_delta` accumulate `partial_json` into a per-index string
  buffer, parsed once at `content_block_stop`/`message_stop` into the
  `ToolCall.arguments` value (same "accumulate-then-parse" pattern OpenAI's
  tool-call handling already needs). On `message_delta`/`message_start`
  capture `usage.{input_tokens,output_tokens}`. Stop on `message_stop`.
- **Not stealing**: adaptive/legacy thinking-budget branching,
  `AnthropicThinking`/`AnthropicOutputConfig`, prompt-cache
  `cache_control`/TTL logic, Kimi/SAP-adjacent auth detection, image content
  blocks (Pii's `ChatMessage` has no image field today).

### 3.6 Spec-string grammar and `provider_from_env`

Extend `crates/pii-model/src/lib.rs`'s `provider_from_spec`/
`resolve_with_transport` dispatch, following the existing `ollama:<model>`
precedent (`ollama::provider_from_env`, `crates/pii-model/src/ollama.rs:359`):

```
openai:<model>                          → base_url = https://api.openai.com/v1, model = <model>
openai:<base_url>:<model>               → explicit base_url (DeepSeek/Groq/Together/vLLM/llama.cpp/…)
anthropic:<model>                       → base_url = https://api.anthropic.com/v1/messages, model = <model>
anthropic:<base_url>:<model>            → explicit base_url
```

Grammar note: `<base_url>` itself may contain `:` (scheme separator,
optional port), so parsing must **split from the right** — take the last
`:`-separated segment as `<model>`, everything before the *first* `:` as
`<kind>`, and the remainder as `<base_url>` (`spec.split_once(':')` for
kind, then `rest.rsplit_once(':')` for base_url/model when `rest` contains
another `:` after stripping a `scheme://` prefix — needs a small dedicated
parser, not a naive `split(':')`, precisely because URLs contain colons).
A concrete parse routine:

```rust
fn split_base_url_and_model(rest: &str) -> (Option<&str>, &str) {
    // rest is either "<model>" or "<scheme>://<host...>:<model>"
    match rest.rfind("://") {
        None => (None, rest), // no scheme => rest is just <model>
        Some(_) => {
            let last_colon = rest.rfind(':').expect("scheme guarantees a colon");
            (Some(&rest[..last_colon]), &rest[last_colon + 1..])
        }
    }
}
```

Each kind's `provider_from_env` reads its API key from the conventional env
var (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`) via `pii_core::env_var`, exactly
like `ollama::provider_from_env` reads `OLLAMA_API_KEY`/`OLLAMA_HOST` — no
new config plumbing needed, this is a drop-in extension of the existing
pattern. Both new `provider_from_env` functions live in their own modules
(`openai::provider_from_env`, `anthropic::provider_from_env`) and are wired
into `resolve_with_transport` next to the existing `ollama:` branch:

```rust
pub fn resolve_with_transport(spec: &str, transport: Arc<dyn HttpTransport>)
    -> Result<Arc<dyn ModelProvider>, PiiError> {
    if spec.starts_with("ollama:") { return ollama::provider_from_env(spec, transport); }
    if spec.starts_with("openai:") { return openai::provider_from_env(spec, transport); }
    if spec.starts_with("anthropic:") { return anthropic::provider_from_env(spec, transport); }
    provider_from_spec(spec)
}
```

### 3.7 Routing through `pii-net`'s grant/egress chokepoint

No new mechanism needed — this is exactly the seam `OllamaProvider` already
uses: the provider holds `Arc<dyn pii_core::HttpTransport>` and never
touches a socket. The *caller* that constructs a `NetService` (the real
`HttpTransport` impl in `pii-net::service`) is responsible for the egress
policy/grants (`crates/pii-net/src/grant.rs`'s `HostScope`/`NetGrant`) that
authorizes `api.openai.com`/`api.anthropic.com` as reachable hosts for the
`model` capability. This is operator/deployment configuration (which hosts a
given `CapSet` may dial), not something `pii-model` needs to know about —
same as today, where nothing in `pii-model` allow-lists `127.0.0.1:11434`
for Ollama; that host is authorized wherever the daemon wires up its
`Grants`/`CapSet`. Worth flagging in the eventual spec's exit criteria: the
default/example grant config should gain `api.openai.com:443` and
`api.anthropic.com:443` entries (or a wildcard note) so the feature is
reachable out of the box, mirroring wherever Ollama's default host is
currently allow-listed.

## 4. What to steal vs. leave behind — summary table

| Steal | Leave behind |
|---|---|
| `OPENAI_API_URL`/`ANTHROPIC_API_URL` constants and the `with_base_url` builder pattern | Async `Provider`/`#[async_trait]`/`futures::Stream` — Pii is sync via `HttpTransport::request_streaming` |
| Request/response field names and shapes (`OpenAIRequest`, `OpenAIMessage`, `OpenAITool`, `AnthropicRequest`, `AnthropicMessage`, `AnthropicContent`, `AnthropicTool`) | `Cow<'a, str>` zero-copy borrowing — not worth the lifetime complexity for Pii's request volume; plain owned `String`/`Value` like `OllamaProvider` |
| SSE frame parsing strategy (buffer partial reads, split on frame boundary, forward deltas incrementally) — but reimplemented against `HttpTransport::request_streaming`'s sync chunk callback, not `SseStream`/`eventsource` crates | pi_agent_rust's actual `SseStream` type (built on `reqwest::bytes_stream()` + tokio) |
| Tool-call streaming accumulation patterns: OpenAI's by-index upsert across chunks, Anthropic's `content_block_start`→`delta`→`stop` typed lifecycle | `complete_partial_json`'s live-partial-JSON-closing (UI-only concern, not needed since Pii's `on_delta` only carries text, not partial tool args) |
| `x-api-key`/`anthropic-version`/`Authorization: Bearer` header assembly, `is_first_party_anthropic_base_url`-style base-URL detection (defer implementing, but keep the technique in mind for prompt caching later) | OAuth token detection, SAP AI Core credential resolution, Kimi device-id/OAuth, DeepSeek `reasoning_style`, OpenRouter `HTTP-Referer`/`X-Title`/routing overrides, adaptive-vs-legacy-thinking budget logic, prompt-cache `cache_control`/TTL/beta-flag wiring, Azure-specific `max_completion_tokens` field-name compat |
| `convert_tool_to_anthropic`, `convert_message_to_anthropic`'s per-role match arms (as a reference for Pii's much simpler `ChatMessage`→wire mapping) | pi_agent_rust's richer `Message`/`ContentBlock` enum (multi-block assistant messages, images, thinking blocks with signatures) — Pii's `ChatMessage` is flatter and doesn't need this yet |

## 5. Recommendation

1. **Fold Ollama into the generic OpenAI provider, don't keep it separate
   long-term.** `OllamaProvider` already speaks the OpenAI-compatible dialect
   (`/api/chat` vs `/chat/completions`, `options.num_predict`/`num_ctx` vs
   top-level `max_tokens`/no context-window param — these are the *only*
   real differences). Once `OpenAiProvider` exists with a configurable
   `base_url` and path suffix, `ollama:` becomes a thin preset:
   `OpenAiProvider::new(...).with_base_url(OLLAMA_HOST).with_path("/api/chat")
   .with_num_ctx(...)`. Do this as a **follow-up** refactor after
   `OpenAiProvider` ships and is tested against real OpenAI — don't couple
   the two in one spec, since Ollama's NDJSON-not-SSE streaming and
   `num_ctx` option are genuine (small) dialect differences worth an
   explicit compat toggle rather than silently forcing Ollama through the
   SSE path. Until that refactor lands, keep `OllamaProvider` as-is; it has
   passing tests and non-test call sites depend on its exact shape.

2. **Ship in two specs, not one**: Spec A = `OpenAiProvider` (new file
   `openai.rs`, `sse.rs`, spec-string grammar for `openai:`), since it's the
   simpler shape and covers the most providers (DeepSeek, Groq, Together,
   vLLM, llama.cpp all "just work" via `base_url`). Spec B =
   `AnthropicProvider` (new file `anthropic.rs`, reusing `sse.rs`), which
   needs the `system`-field/tool_result/tool_use projection logic that has
   no OpenAI analog. Both specs should require: unit tests with a
   `RecordingTransport`/`ChunkedTransport` fixture (following the existing
   pattern in `pii-model/src/lib.rs`'s `contract` module and
   `ollama.rs`'s `streaming_tests` module) proving (a) the non-streaming
   request body shape, (b) streaming text deltas fire incrementally and in
   order, (c) tool calls round-trip through both the buffered and streaming
   paths, (d) a split-mid-JSON SSE frame across two `request_streaming`
   chunks is still parsed correctly (mirroring
   `ollama_streaming_survives_split_tool_call_line`).

3. **Do not implement prompt caching, thinking/reasoning dialects, or
   OAuth in the first cut.** They're real pi_agent_rust features but
   orthogonal to "can Pii send a chat request to any OpenAI/Anthropic-shaped
   endpoint" — adding them now would triple the diff size for capability Pii
   doesn't yet have a caller for (no `ThinkingLevel`/`CacheRetention`
   equivalent exists in `pii-model::ChatRequest` today). Flag prompt caching
   specifically as valuable future work once `AnthropicProvider` exists,
   since it's a pure cost optimization with no behavior change to callers.

4. **`max_tokens` default and `num_ctx`**: Anthropic requires `max_tokens`;
   default it in the provider (not the caller) exactly as `OllamaProvider`
   already defaults `num_ctx` when `ChatRequest.num_ctx == 0`. OpenAI's
   `max_tokens` is optional server-side, but Pii should still send a
   provider-level default (steal `DEFAULT_MAX_TOKENS`) for predictable
   billing/latency, matching Ollama's existing behavior of always sending
   `num_predict`.

5. **No `pii-net` changes required.** HTTPS/TLS (spec 17) and
   `request_streaming`'s chunked-byte callback already provide everything
   both providers need. The only operational follow-up is adding
   `api.openai.com:443`/`api.anthropic.com:443` (or an equivalent wildcard)
   to whatever default `Grants`/`HostScope` configuration ships with the
   daemon, so the new providers are reachable without a manual grant edit.
