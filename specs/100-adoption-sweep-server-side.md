# Spec: 100 — adoption sweep, part 1: the server-side subsystems actually emit

**Project**: <repo>
**Phase**: P0/P1 observability. Diretiva: *"we must enforce observability! via traits on every crate!"*
Contract: spec 97 (`pii-observe`). Enforcement shell: spec 98 (`lint-observability`). Design:
`_docs/06-research/053-…md` §3–§4.
**This spec removes the PLACEHOLDER exemptions for the eight subsystems that have something real to
observe.** The clients/channels group is spec 101; the enforcement hardening (exemption registry,
per-file print budget, fail on unsanctioned exemptions) is spec 102 and lands only after both sweeps,
so the gate stays green at every step.

## Context
`lint-observability` currently prints `19 crates exempt, 0 findings` because spec 98 added
`// observability: none — pending the spec 100 adoption sweep (no seam yet)` to every crate. This spec
converts the eight crates whose internals are worth observing and deletes their placeholder line.

## Requirements
R1. **Convert these eight crates** — for each: (a) add the `pii-observe` dependency, (b) give its public
    types the `Observable` seam (or take `Arc<dyn Observer>` at construction — prefer constructor
    injection: a caller then cannot build the subsystem without an observer), (c) emit **at least one
    real event or measure** at its boundary, using `pii_observe::names` consts — never string literals
    at the call site, (d) DELETE its `// observability: none — …` line, (e) implement `Health` where it
    owns subsystem identity:

    | crate | the observation to add (at minimum) |
    |---|---|
    | `pii-store` | persist failures already route through an event — emit it through the contract; add a `Health` impl reporting path/bytes/format_version/entries (spec 99 built `StoreInfo` — wire it) |
    | `pii-net` | egress denials and dial/accept failures as events; `Health` with listener identities (already exists in `StatusResult`) |
    | `pii-tools` | a per-call event on every tool invocation (tool name + outcome) and a deny-list hit event |
    | `pii-model` | provider-in-force + request latency + provider error events; `Health` naming the provider and model actually resolved |
    | `pii-worker` | supervisor state transitions (spawn/ready/draining/restarting/failed) as events |
    | `pii-graph` | already emits metrics (`graph.node.dead`, `graph.join.unsatisfiable`, `graph.batch.width`) — route them through the contract and drop its placeholder |
    | `pii-memory` | search/consolidate outcome events + `Health` reporting doc count and index size |
    | `pii-server` | already uses the contract — drop its placeholder, and register the subsystems it aggregates (see R2) |
R2. **A named registry, pinned by a test** (the user's "Java interface" question, answered structurally):
    the daemon's health aggregator must hold its subsystems as `&dyn pii_observe::Health` and a test must
    assert the registered set **by name** (e.g. `["store","memory","net","worker","channels","graph"]`).
    Mutation control: removing one registration turns that test RED (paste it).
R3. **An adequacy floor**: one passing test per converted crate asserting a **named** event or measure
    from it (use `pii_observe::names` consts, not literals). The report must include a table
    `crate → test name → the name asserted`, and the tests must actually run (paste the run).
R4. **Do not add any new raw print** anywhere, and do not add entries to the print allowlist.
R5. `specs/` is read-only for you.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- lint-observability` exits 0 and its summary shows the exempt count DROPPED by exactly the eight crates converted here (paste both the earlier `19` and the new number)
- [ ] `[ "$(grep -rn '// observability: none' crates/pii-store crates/pii-net crates/pii-tools crates/pii-model crates/pii-worker crates/pii-graph crates/pii-memory crates/pii-server | wc -l)" = 0 ]`
- [ ] the adequacy floor table (`crate → test → name`) is in the report, and each listed test passes when run (paste the command and output)
- [ ] the registry test exists, asserts the subsystem set by name, and turns RED when one registration is removed (paste both runs)
- [ ] `cargo test --workspace --lib --bins --tests` exits 0; `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0; `lint-files` reports no file over the warn threshold
- [ ] the report lists every file changed and states, per crate, whether an observer is injected at construction or exposed as a handle (and why, when the crate has no constructor)

## Guardrails
- **A seam that nothing exercises is the defect class this repo keeps finding**: every converted crate
  needs the R3 test. Do not add an event and leave it untested.
- Do NOT touch the exemption mechanism or the print allowlist (spec 102 does that) and do NOT remove
  another crate's placeholder.
- Do NOT change behaviour: events are additive; no existing test's expectation may change, and no error
  may be swallowed into an event where it was previously returned.
- Do NOT log secrets, tokens, env values, or file contents; redaction is spec 103's subject.
- Do NOT edit files under `specs/`.
- Scratch under `<scratch>/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
Landed: 8 crates now emit through the contract — `pii-store` (persist failures + `Health`),
`pii-net` (`NET_EGRESS_DENIED`, `NET_ACCEPT_FAILED`), `pii-tools` (`TOOLS_CALL`,
`TOOLS_DENY_LIST_HIT`), `pii-model` (`MODEL_PROVIDER_IN_FORCE`, `MODEL_REQUEST_LATENCY`,
`MODEL_PROVIDER_ERROR`), `pii-worker` (`WORKER_STATE_CHANGED`), `pii-graph` (`GRAPH_BATCH_WIDTH` through
the contract), `pii-memory` (`MEMORY_SEARCH`, `MEMORY_CONSOLIDATE` + `Health`), `pii-server` (the
aggregator). Both gates green.

**A correction to MY OWN numbers, from the worker (and it is right):** the exempt count was **15 before**
this spec, not the `19` I had been quoting, and it is **11 after**. Spec 99 (health identity) had already
taken `pii-store`, `pii-net`, `pii-worker` and `pii-server` off the exempt list by giving them real
contract use, so only four crates still carried the placeholder when this spec started. Its drop is 4,
and 8 crates gained emissions. I had been repeating a stale figure; recorded here rather than quietly
restated.

**Verified by the orchestrator (not taken on report):**
- `lint-observability` → 11 crates exempt, 0 findings; the list is exactly the four sanctioned
  (`pii-abi`, `pii-proto`, `pii-uds`, `pii-observe`) plus the seven spec 101 will convert.
- The adequacy floor runs: `pii-tools --lib call_and_deny_list_hit_reach_the_observer_by_name` ok;
  `pii-graph --test observe_contract` ok; `pii-memory --test observe_contract` (2) ok.
- **Mutation control A** (mine): renaming the emitted tool event to `tools.call.MUTANT` turns the
  adequacy test RED (`assertion left == right failed`). Restored.
- **Mutation control B** (mine, and this is the answer to the user's "Java interface" question, proven
  structurally): removing the store registration from the aggregator turns `registry_is_pinned_by_name`
  RED —
  `left: ["memory","net","worker","channels"]  right: ["store","memory","net","worker","channels"]`.
  Restored.
- `quality-gate.sh` ALL PASS (incl. `PASS: lint-observability`) · `fresh-gate --worktree` PASS.

**Shortfalls, as the worker reported them:** the pinned registry set has no `graph` (it owns no identity,
so it implements no `Health` — a judgement, not an omission, and spec 102's registry must say so
explicitly), and the group-line deletion control was not exercised separately.
