# 02-guides

How-to guides: the pidag way of work, feature gating, the MCP bridge, local llama.cpp.

- `quality-gate-and-fresh-tree.md` — the gate manual: the eight steps, the two-tree model, and the `tmp_scratch` rule.
- `writing-a-channel.md` — the `Channel` trait contract, config section, events, and a copyable adapter skeleton (spec 112).
- `writing-an-sop.md` — the SOP file format, the compilation rule, precondition semantics, the approval flow, and the worked example (spec 113).
- `model-tiers.md` — `[[model_tier]]` keys, the model precedence, the capability-layer tool gate, and the no-downgrade rule (spec 114).
- `learning-loop.md` — extraction and verification, the first two stages of the learning loop: the four extractors and their thresholds, refusal rules, the data-not-instructions stance, read bounds (spec 123); the isolated shadow-run verifier, its budgets and three verdicts (spec 124); and promotion — the one-kind whitelist, ledger, rollback, TTL, measured outcome and automatic reversal (spec 125).
- `wasm-plugins.md` — the plugin ABI, the four runtime-enforced limits and their defaults, the `[wasm]` keys, what a plugin can and cannot do, how to build one, and what a wasm sandbox is not (spec 122).
- `hardware-hal.md` — the HAL traits, the pin/bus capability scope, the `[hal]` keys, the simulated backend, and the real backend's UNVERIFIED status with what would verify it (spec 121).
- `mqtt.md` — `[[mqtt]]` keys, the payload mapping, the `mqtt` publish node, backoff, the plaintext-credentials rule (spec 117).
- `memory-entities.md` — typed entities and relationships with provenance: ids, merge, the integrity rule, caps and the refuse-never-evict policy, bounded queries, the `pii memory` entity commands, the `remember`/`recall` nodes and their capabilities (spec 126).
- `discord-graphs.md` — the `post` egress node (capability, idempotence key, bounds, what may be posted), the `classify` node, `graphs/discord-triage.toml` and `graphs/discord-digest.toml`, and the human tag step (spec 128).
