# M7 — retiring the dormant surface (the six 059 decisions)

**Date**: 2026-10-14 · **Status**: in progress, one of six needs a host action · **Source**: Opus 059 §2.6
**Why this file exists**: 059 measured one pillar with a real user (Hermes → `pii-mcp`, 184 calls in 30 days =
0.32% of Hermes tool messages) and pillars 2/3/4 with none. Building more surface for zero users is the trap this
list closes. Each row states the measured usage, the decision, and who acts.

| surface | measured usage on this machine | decision | who acts |
|---|---|---|---|
| `gemini` model fallback | removed 2026-10-13 (a fallback chain reaching Google; no Google call was wanted) — verified by the next run's log: `Fallback activated: glm-5.3-flash:cloud → deepseek-ai/deepseek-v4.1-flash`, zero Google calls | **RETIRED** | done |
| audit trails A/B/C (059 §3) | option B (single writer) implemented as spec 138; A and C were evaluated and dropped, not deferred | **CLOSED** | done |
| `telegram` / `matrix` / `webhook` channel kinds | 0 channel posts ever; the only live channel is the Discord forum (+ `cli`) | **DORMANT** — keep the code, remove it from the default config surface, state the count | spec 155 (implemented): `examples/pii.production.toml` names `cli, discord` as the default kinds and records "0 posts ever"; the kinds stay in code |
| `gpiod` HAL (`hal-gpiod` feature) | 117 lines that have never run on hardware; feature off by default; the gate carries a `clippy(hal-gpiod)` step for code nothing executes | **DORMANT** — keep behind the feature flag, stop counting it as coverage, note that it is unexercised | spec 155 (implemented): `quality-gate.sh --summary` and the end of every gate run print `UNEXERCISED: clippy(hal-gpiod)`; the step still runs |
| local `ollama.service` (host) | installed on the host, bound 127.0.0.1:11434, measured unreachable from the container (`000` on both `lnx:11434` and `10.10.10.217:11434`); nothing here depends on it | **MUST NOT BE IN THE FINAL VERSION** (user directive 2026-10-10: "ollama wont be part of our setup! only llama.cpp") | **user — host action** (`systemctl disable --now ollama.service`) |
| `pii-mcp` binary the Hermes install uses | the installed binary is from 17 September (86 commits behind) and is the ONE surface with a real user | **SWAP AFTER the deploy** — behind its own probe, never before: it is the only load-bearing thing in the list | mine, after M5 |
| Discord forum as home channel + default cron tag | forum `1558589728671146105` is the conversation surface; `DISCORD_*` still name the text channel `hermes` | **DECIDED**, not switched (switching removes the only place an outage can be heard) | recorded |

## What is left of M7

1. `telegram`/`matrix`/`webhook`: remove them from the default channel config (keep the kinds), and add one line
   stating "0 posts ever" so the next reader does not mistake their presence for use.
2. `gpiod`: mark the feature unexercised in the gate's own summary, so its steps cannot read as coverage.
3. `ollama.service`: **the user's**, one command on the host — the only M7 item that is not mine.
4. `pii-mcp` swap: sequenced deliberately AFTER the deploy, because it is the one surface with a live user.

## The rule this list enforces

A surface nobody uses is not neutral: it costs a build step, a gate step, a maintenance path, and — worst — it makes
a green gate look like a working product. 059's verdict is the one to keep in view: *the tree delivers working
substrate for the four pillars; the machine delivers almost none of it.* Retiring the dormant surface is what keeps
that sentence from becoming permanent.
