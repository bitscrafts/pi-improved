# Spec: 122 — the WASM plugin host: third-party nodes without rebuilding pii

**Project**: /projects/pi-improved
**Phase**: **Stage B** of `057` (vision phase 4's third component: *"WASM plugin host"*). It is also the surface
with the worst failure mode in this whole plan: it runs **untrusted code inside the daemon**.

## What exists (measured 2026-10-10)
- **Nothing**: `grep -rn wasmtime crates/*/Cargo.toml Cargo.toml` → no match; `grep -n 'name = "wasmtime"' Cargo.lock`
  → no match. The runtime is not in the tree.
- The repo **anticipated** it: `xtask/src/deps.rs` lists `wasmtime` in `FORBIDDEN_TRANSITIVE` (line 15, the trees
  that must stay runtime-free) and in `SINGLE_VERSION` (line 34, exactly one version in the resolved graph). So the
  new dependency has to land inside **one** crate and the invariants have to be told about it deliberately — a
  `deps-check` finding here is the check doing its job, not an obstacle to route around.
- The capability model attenuates paths, tools and (spec 121) pins; the observability contract is enforced by the
  gate (spec 118) with the lint's completeness rule over `ALL`.

## Requirements
R1. **A new crate `pii-wasm`** holding the host; `wasmtime` may appear **only** there, and the version must stay
  single. If `deps-check` needs an explicit allowance for the new crate, add it **with a stated reason** — never by
  removing a rule or widening `FORBIDDEN_TRANSITIVE`. State the wasmtime version and what enabling it costs
  (build time, binary size).
R2. **A `wasm` node kind**: `module = <path>`, optional `entry` (default a documented name), input = the node's
  input JSON, output = the value the module returns, validated as JSON (a module returning garbage fails the node
  with a named error, it does not corrupt the blackboard).
R3. **The host must be safe by construction, all four enforced, each with its own test:**
    a. **epoch interruption** with a per-call wall-clock budget (a module with an infinite loop is interrupted, and
       the node fails with a named `timeout` reason — paste the elapsed time);
    b. **fuel** metering, opt-in per node with a default ceiling when enabled (a module that burns fuel is stopped);
    c. **memory limit** per instance (a module that tries to allocate without bound is refused, not OOM-killed);
    d. **no ambient authority**: no WASI preopens, no filesystem, no network, no env — a module that tries to open a
       file fails. Say exactly which WASI functions are exposed (ideally none beyond what the ABI needs).
R4. **Module path scope**: a module may only be loaded from a configured directory (a `[wasm]` section with a
  `dir`, off by default like the HAL) — a path outside it is refused. A `wasm` node with no `[wasm]` config fails
  loudly with the reason.
R5. **Observability**: named events for load / instantiate / call / trap / timeout / fuel-exhausted / refusal, each
  carrying the module path and the entry, registered in `ALL`, plus counters in `pii metrics` (calls by outcome,
  refusals). A trap must be reported with the module and the wasm backtrace, not swallowed.
R6. **Tests, all offline** with checked-in fixtures (hand-written WAT compiled at test time, or tiny `.wasm` files):
  echo round-trip; infinite loop interrupted; fuel exhausted; memory limit hit; a filesystem-open attempt refused;
  a trap reported; an out-of-scope module path refused; a module returning invalid JSON failing the node. Each
  refusal/limit case needs a **mutation** that turns it red (e.g. remove the epoch deadline, remove the memory
  limit, add a preopen) — paste each.
R7. **Docs**: `_docs/0X-guides/wasm-plugins.md` — the ABI, the four limits with their defaults, the config keys,
  what a plugin can and cannot do, and how to build a plugin. `001-pii-vision.md`'s phase-4 line updated.
R8. **Honest statement of what a WASM sandbox is not**: it is a security boundary for the module, not for the host;
  a bug in wasmtime is a bug in pii; and nothing here has been fuzzed or adversarially tested beyond the cases above.

## Exit Criteria
- [ ] `cargo test -p pii-wasm` exits 0 with R6's cases; each limit/refusal has a mutation pasted red then restored
- [ ] the infinite-loop case pastes the elapsed wall time proving the epoch deadline fired
- [ ] `cargo run -q -p xtask -- deps-check` exits 0 with the new allowance stated in the diff; `net-check`,
  `doc-drift-check` and `lint-observability` still exit 0
- [ ] the counters move in `pii metrics` and the names are in `ALL` (paste)
- [ ] `pii health` reports the wasm subsystem, or the report says why not
- [ ] a graph runs a wasm plugin end to end and the journal shows the returned value (paste)
- [ ] the guide exists; `001`'s phase-4 line updated (paste it)
- [ ] `bash quality-gate.sh` exits 0 (14 steps, or 15 if you add one for the wasm feature — say which);
  plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists files, names, config keys, the wasmtime version and its cost, and what was NOT tested

## Guardrails
- **No ambient authority by default.** Adding a preopen, an env var or a WASI socket for convenience is a fail.
- Every limit must be **enforced by the runtime**, not by asking the module politely (no "we check the loop count
  inside the plugin").
- Never remove or widen a `deps.rs` rule to make the new dependency pass; add a narrow, stated allowance.
- A malicious module must never take the daemon down: no `unwrap` on a guest-controlled value, no panic on a trap.
- Do not add a second wasm runtime; one version, one crate.
- Scratch under `/projects/_tmp/`, delete your own, no daemon left running. Do NOT commit.
