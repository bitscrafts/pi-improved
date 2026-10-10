# Dormant capabilities — what is deliberately NOT wired, and why

**Purpose**: capabilities that EXIST in the tree but are deliberately not reachable, each with the reason and
the exact mechanism that keeps it dormant. "Dormant" is a decision with a check behind it, not a forgotten
crate. When the owner reverses a decision here, the entry moves to the roadmap and the check is removed in
the same commit.

## 1. `pii-http-fetch` — outbound HTTP for the agent — DORMANT (owner decision, 2026-10-10)

**What it is**: `crates/pii-http-fetch` — "the allowlist is the feature: GET, no headers, no body"
(`src/fetch.rs:3`). It grants egress per host (`http:<host>` capability), has an audit path
(`src/audit.rs`), and returns `PiiError::Denied` outside the allowlist.

**Decision (owner, verbatim)**: *"also on pii-http-fetch document it and Leave it dormant for now."* — the
capability stays in the tree, unreachable from an agent turn.

**Why dormant**: giving the agent outbound HTTP is a **capability decision, not an observability one** (spec
109's own words). The crate is not a missing feature; wiring it decides that a model-driven turn may reach
the network with a host allowlist as the only boundary, and that call belongs to the owner.

**How it is kept dormant (the check, not the intention)**:
- no crate depends on `pii-http-fetch` (verified 2026-10-10: `cargo tree` shows no dependents);
- `crates/pii-http-fetch/tests/dormant.rs` **fails** if any crate starts depending on it (added by spec 109);
- the crate is NOT exempt from `lint-observability`: it already uses the `pii-observe` seam, so the exemption
  would have been false (spec 109 R5).

**What reversing it would take** (so the decision is cheap to take later):
1. a grant key per host (`http:<host>`) declared in the run/scope, not a global allow-all;
2. the tool surface (`http_fetch`) exposed with the allowlist enforced IN the tool, and a denial event
   (`core.cap.denied` + the egress event) — the seam already carries both;
3. a decision on redirects, TLS, size caps and timeouts — none of which the crate documents today;
4. the audit record persisted (the rotated audit log from spec 107 is the natural home);
5. the owner's explicit "wire it", and the removal of `tests/dormant.rs` in the same commit.

**Status**: dormant, documented, checked. **Owner action needed to change it: one word.**

## 2. Other capabilities that are absent (not dormant — never built)

These are listed so the distinction is clear: "dormant" means the code exists and is withheld; "absent" means
it does not exist yet and belongs to the roadmap (`_docs/06-research/052-…md` P4 = BUILD, decided
2026-10-10):

| capability | state | home |
|---|---|---|
| WASM host (`wasmtime`), MQTT/HAL/GPIO/SOP nodes | **absent** | pillar 2 (assistant/gateway + IoT/automation), vision `001` |
| channels beyond `cli` and `discord` | **absent** (2 exist) | pillar 2 |
| provider kinds beyond `scripted`/`llamacpp-server`/`llamacpp` | **absent** (llamacpp off by default) | pillar 2 |
| entity/relationship memory model, tiering | **absent** | pillar 3 |
| skill extraction, procedural templates, verifier-gated promotion | **absent** (the trajectory substrate landed as spec 103) | pillar 4 |
| `seccomp` syscall filter | **absent, documented** | runbook (spec 107 R6) |
