# Spec: 117 — MQTT: the reactive pillar's front door

**Project**: /projects/pi-improved
**Phase**: **Stage B** of the vision plan (`_docs/06-research/057-vision-implementation-plan-2026-10-10.md`),
first row. Vision phase 4: *"IoT/reactive (MQTT, hardware HAL, WASM plugins)"* — today the graph can be driven
by a CLI, a client socket and an HTTP webhook, but **nothing can be driven by a device**.

## What was measured before writing this (2026-10-10)
- **No broker on this host**: `which mosquitto mosquitto_sub mosquitto_pub` → nothing. So the tests must bring
  their own broker in-process; a test that needs a system broker is a test that never runs here.
- **crates.io is reachable and the client exists**: `cargo search rumqttc` → `rumqttc 0.25.1`. The broker crate
  in the same family (`rumqttd`) is the natural **dev-dependency** for an in-process loopback broker.
- Dependency policy of this lane: a new dependency is allowed when it is stated and justified (spec 111 added
  none; MQTT is the case where hand-rolling a 3.1.1 client buys nothing but bugs). **State the crates, versions
  and why** — and if you choose to hand-roll, say what that costs instead of silently growing the surface.

## Requirements
R1. **A `mqtt` node kind.** Two directions, both real:
    a. **Subscribe → run**: a node subscribes to a topic filter and, on a message, starts/marks the run with the
       payload as its input. Say how the payload becomes the input (documented mapping: JSON parsed, else text).
    b. **Publish**: a node publishes a topic + payload derived from its predecessor's output, with QoS and
       retain stated explicitly (never left implicit).
R2. **Config**: an `[[mqtt]]` (or equivalent) section per broker: URL, client id, topic filters, QoS, and
    credentials **never from the config file** — an env var or a 0600 file, reusing `pii-channels/src/secret.rs`.
    The client id must default to something unique per instance so two instances do not fight over one id.
R3. **No silent degradation.** Broker unreachable → the node's channel reports **Degraded with the reason** in
    `pii health --json`; a named event is emitted; reconnect uses a **bounded exponential backoff** with a
    stated ceiling (an unbounded hot reconnect loop is the failure mode this requirement exists to prevent).
    A message dropped because no handler was ready is **counted**, not forgotten.
R4. **Credentials are a security surface.** (a) They never appear in an event, error, log or trajectory (extend
    the spec-107 canary path to this node if needed). (b) **Refuse** to send a username/password to a
    **non-loopback plaintext** (`mqtt://`) broker unless an explicit opt-in config key is set with a warning —
    do not silently leak a password in clear text. State TLS support (or its absence) in the report and runbook.
R5. **Tests, with a broker that runs here**: an in-process broker bound to a loopback port, with tests for —
    subscribe→run fires with the payload; publish→the broker receives exactly the topic+payload (QoS/retain
    asserted); broker down → Degraded + event + bounded reconnect attempts (assert the COUNT, not just "it
    retried"); credential-over-plaintext-to-a-non-loopback-host refused (the negative control: loopback allowed);
    a malformed payload does not panic and is reported.
R6. **Observability**: named events for connected / connection lost / message received / message dropped /
    published, registered in `ALL` (the lint's completeness rule covers them), with the topic in the fields —
    plus counters (received, dropped, published) visible in `pii metrics`. Spec 110's lesson: a surface that
    cannot be seen under load is a surface that will be missed.
R7. **Docs**: `_docs/0X-guides/` a guide for the node (config keys, payload mapping, QoS/retain, backoff, the
    plaintext rule) + a sample config + a runbook line naming the broker dependency and how to point it at a
    real broker. `001-pii-vision.md`'s IoT line moves from ABSENT to what actually exists (and says what does not).

## Exit Criteria
- [ ] a real message published through the in-process broker fires a run carrying the payload (paste the run id,
      the journal line, and the broker's view of the message)
- [ ] a publish node's message arrives at the broker with the asserted topic, payload, QoS and retain flag (paste)
- [ ] broker down: health says Degraded with the reason, the event fires, and the reconnect count is asserted
      (paste the count) — and the process does not spin (paste something that shows the backoff, e.g. attempts)
- [ ] credentials: canary in neither events nor logs nor trajectory (paste the counts); a plaintext non-loopback
      broker with credentials is REFUSED without the opt-in (paste the refusal), and the loopback case works
- [ ] the new name consts appear in `ALL` (paste the grep); metrics counters respond (paste)
- [ ] the guide, the sample config, the runbook line and the `001` line exist (paste paths + the `001` line)
- [ ] `bash quality-gate.sh` exits 0; `fresh-gate --worktree` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the report lists crates+versions added and why, files, config keys, event names, and what it did NOT verify

## Guardrails
- **The broker in the tests must be local and in-process.** Never point a test at a public broker, and never
  require a system service to exist.
- Never log, emit or persist a credential; the plaintext rule is a refusal, not a warning comment.
- No unbounded queues or hot reconnect loops; every bound gets a named event when it is hit.
- Do not add a dependency for the HAL of a second purpose — this spec is MQTT only.
- Scratch under `/projects/_tmp/`, delete your own, no broker or daemon left running. Do NOT commit.

## Orchestrator verification + a cross-invariant conflict found and fixed (2026-10-10)
**My controls (three mutations, each restored):**
- **The credential refusal bites**: disabling it (`if false && has_creds && !is_loopback(&host) && !allow_plaintext…`)
  → `credentials_to_a_plaintext_non_loopback_broker_are_refused_unless_opted_in` **FAILS**; restored → 12 pass.
- **The backoff bound bites, with the number**: making `backoff_ms` return 0 → both
  `backoff_doubles_to_a_ceiling_and_never_overflows` and `broker_down_is_degraded_with_reason_event_and_bounded_attempts`
  FAIL, the latter printing **`bounded exponential backoff: 5435 attempts in 500ms`** — the hot retry loop the
  requirement exists to prevent, caught with its rate. Restored → 12 pass.
- Tests I ran: `pii-channels --test mqtt` **12 pass**; `pii-server --test mqtt_e2e` **5 pass**.
- No stash was left behind by the worker's `git stash` (checked `git stash list`: empty; the 27 dirty paths are all
  its work).

**A REAL CONFLICT between two project invariants, found by an ungated check.** `cargo xtask deps-check` failed:
`pii-proto: direct dependency 'pii-observe' is not in the allowlist`. That is not the worker's fault — it is
**spec 109's payoff colliding with the repo's own invariant 2** (`xtask/src/deps.rs:18-24`: "`pii-proto`'s direct
non-optional deps must be exactly serde/serde_json/schemars/thiserror"). Spec 109 R7/R8 made the handshake client
emit, which added the contract as a hard dependency of the wire crate.
**Fixed by the orchestrator in three lines, preserving BOTH intents**: `pii-observe` becomes an **optional** dep of
`pii-proto` and joins the existing `client` feature (`client = ["dep:tokio", "dep:pii-observe"]`). The default build
stays serde-only (the invariant's actual purpose — the wire types pull no runtime), the client still emits, and
every consumer already enables `client` (`pii-cli`, `pii-evolve`, `pii-server`, `pii-tools`, `pii-tui`, `pii-web`).
Verified: **`deps-check` rc=0**, workspace `cargo check` clean, `lint-observability` → `registry size 2, 0 findings`,
`fmt --check` clean.

**Two more ungated checks fail on the untouched tree — one of them a false positive:**
- `net-check` reports `xtask/src/lint_observability.rs:27 — UnixStream::connect`: line 27 is a **DOC COMMENT**
  describing R8's exemption shrink. The check scans source text without excluding comments, so it flags its own
  documentation. A check that reports a violation that does not exist is the §1 class, and it must be fixed before
  that check could ever be promoted to a gate step.
- `doc-drift-check` reports `_tmp/sidecar.log` (named in `HANDOFF.md`) does not exist — real but trivial drift.

