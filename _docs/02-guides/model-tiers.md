# Model tiers (spec 114)

A **tier** is a named model class — a provider spec plus an optional tool allowlist and token limit.
A graph node asks for one with `tier = "<name>"`; the engine builds one provider per tier at start-up.

## Config

```toml
[core]
model = "llamacpp-server:granite-4.2-3b"  # the run's default

[[model_tier]]
name       = "fast"
model      = "llamacpp-server:qwen2.5-0.5b"  # any provider spec: scripted:, llamacpp-server:, openai:, anthropic:, gemini:
tools      = ["fs_read", "grep"]      # optional: the ONLY tools this tier may call
max_tokens = 512                      # optional: caps max_tokens per request on this tier

[[model_tier]]
name  = "big"
model = "anthropic:claude-sonnet-5-5"  # no `tools` = unrestricted
```

Keys: `name` (unique, non-empty), `model` (required), `tools`, `max_tokens`. Unknown keys and duplicate
names are load errors. A node uses it as `[[node]] kind = "agent"  tier = "fast"`.

## Precedence (one function: `pii_server::tiers::precedence`)

1. the node's explicit `model` (or its `route_step` policy entry)
2. the node's `tier`
3. the run's model (`--model`)
4. `$PII_MODEL`
5. the daemon config's `[core] model`

Levels 3–5 are `Config::resolve_model`; level 4 over 5 is the pre-existing order and is unchanged.
When a node has both `model` and `tier`, the explicit model wins the *model* choice (the run's provider
serves it) but the tier's tool list and limits still bind.

## The tool gate is capability enforcement

A tier with `tools` set runs its agent node under `Ctx::with_tools(list)`: the node's `CapSet.tools`
allowlist is intersected with the tier's (it can only shrink). Every tool call goes through
`Ctx::tool(name)` before dispatch; a tool outside the list returns `PiiError::Denied("tool:<name>")`
and emits a `core.cap.denied` event — whatever the prompt or the model asks. The model is also only
*offered* the allowed tools, but that is a courtesy; the gate is the enforcement.

## No silent downgrade

- Unknown `tier` → the node refuses to build: `model tier 'x' is not configured (available: big, fast)`.
- A tier whose provider cannot be built → `model.tier.unavailable` at start-up, a `model_tier:<name>`
  health subsystem in `Degraded` naming the provider, and `PiiError::Unavailable` for any node that names
  it. It never runs on another tier's model or the default provider. Other tiers are unaffected.

## Observing it

`model.tier.resolved` (`tier`, `model`) is emitted when a tiered node starts, so a trace or trajectory
can be grouped by tier. `pii health --json` lists one `model_tier:<name>` subsystem per tier (identity:
`model`, `tools`).
