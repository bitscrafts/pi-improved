# Spec: 128 — the channel egress node, and the first Discord graphs

**Project**: /projects/pi-improved
**Phase**: after the `057` plan (111–127 complete) — **the user's directive (2026-10-10)**:
*"i think the discord deserves its own graphs"*. The Discord surfaces exist (spec 112: a `discord` channel with
poll **and** send; the Hermes gateway delivers into the forum as thread posts — measured today), but **no graph can
post into a channel**: there is no node kind for egress.

## What exists (measured 2026-10-10)
- `crates/pii-channels/src/discord.rs`: a real `Channel` with `poll()` and `send()`; `Channel` has
  `fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError>` (spec 112).
- Node kinds today: `crash, entity, gate, hal, heal, memory, mqtt, sop, spawn, wasm, worker` — **no channel/post
  node**. MQTT got a `mqtt` node in spec 117 for exactly this reason (publish from a graph); Discord has no
  equivalent.
- Proven today by hand: the gateway delivers into the `pii` forum by creating a **thread post**
  (`adapter.py::_send_to_forum`, verified: thread `1558591788615475311` carries the delivered line). The forum
  currently has **0 tags** and the bot gets `50013 Missing Permissions` when it tries to PATCH `available_tags`
  (a human must create them, or grant Manage Channels).
- `graphs/` holds only `starter` — the domain graphs live as examples (mqtt, hal-blink, memory-recall, soak).

## Requirements
R1. **A `post` node kind (egress to a channel)**, reusing the existing `Channel::send` and the daemon's channel
  registry — do not write a second Discord client:
    a. fields: `channel` (a configured `[[channel]]` name), `to` (destination: a channel/thread id, or a topic for
       forum-style targets), `text` (templated from ports like any node), optional `template` with `${…}`
       substitution of the node's input;
    b. **capability-attenuated**: the node needs a capability scope (e.g. `channel:<name>` or a `post` cap) and is
       refused with a **named** reason outside it (`capability denied: post: channel <name> not granted`);
    c. a `50013`-style provider refusal (permissions), a missing channel, or a bad id must fail the node with the
       provider's message **preserved**, never swallowed;
    d. bounded: a per-call timeout and a response bound; a failure must never hang a run (spec 120's lesson).
R2. **Idempotence knob**: `key = "<stable value>"` so a retried step does not double-post (the engine retries
  nodes; a duplicate post in someone's phone is worse than a failed one). State the mechanism (e.g. remember the
  key in the run journal) and prove a retry posts once.
R3. **Two graphs, committed under `graphs/`** (the user's directive):
    a. `discord-triage.toml` — a message (from the channel ingress) is **classified by rules** (no model needed:
       route on markers/keywords into `question | status | spec | alert | done`), then **posted back into the same
       thread** with the classification, and the run's outcome recorded. It must be runnable with the scripted
       model and produce a pasted journal.
    b. `discord-digest.toml` — gather what the system knows (e.g. `pii learn list` candidates, memory tier counts,
       recent run outcomes through the existing nodes) and post a **digest** into the forum. It must state its
       inputs and produce a pasted journal.
R4. **Tags are a human step, and the spec must say so**: the bot cannot create forum tags (`50013`, measured). The
  guide must give the exact tag set to add in the Discord app (`chat · status · spec · alert · done`, emojis
  optional) and, if the user later wants the bot to auto-apply one, that requires an adapter change (out of scope
  here — file it, do not smuggle it in).
R5. **No live Discord in the gate**: tests use a **mock/loopback channel** (spec 112's pattern) and the `post` node
  must be testable without a token. The Discord-specific path is exercised by a `#[ignore]`d live test with the
  token from the environment, documented as such.
R6. **Observability**: named events `channel.posted` (channel, destination, bytes) and `channel.post_refused`
  (reason), registered in `ALL`, counters in `pii metrics` rendered through a **live daemon**, and the node's
  journal entry carrying the destination so a run can be traced to what someone saw on their phone.
R7. **Docs**: a section in the memory/channel guide (or a new `_docs/0X-guides/discord-graphs.md`) covering the
  `post` node, the two graphs, the ingress→run→post loop, the tag step and the capability needed; `001`'s
  gateway line notes the egress node exists.

## Exit Criteria
- [ ] `cargo test` for the new node: posts through a mock channel; refusal without the capability (named); a
      provider error preserved verbatim; the retry posts **once** (R2); a timeout is bounded (paste the elapsed ms)
- [ ] a mutation per rule: capability check removed; idempotence key ignored; error swallowed into a generic one;
      timeout removed — each pasted red, then restored
- [ ] `graphs/discord-triage.toml` runs end to end with the scripted model: paste the journal showing the
      classification and the post (destination + bytes), and the `channel.posted` event
- [ ] `graphs/discord-digest.toml` runs end to end: paste the journal and the digest text it would post
- [ ] the counters move in `pii metrics` through a live daemon (paste); the names are in `ALL`
- [ ] `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` exit 0 (the new node must not add a
      network call outside the channel — `net-check` is the arbiter)
- [ ] `bash quality-gate.sh` exits 0; plain `fresh-gate` prints `fresh-gate: PASS`; clippy/fmt exit 0
- [ ] the guide states the tag step and the `50013` measurement (paste the line); `001` updated
- [ ] the report lists files, names, the tag ids expected, and what was NOT verified (a live post, unless the
      token is present)

## Guardrails
- **One Discord client.** The `post` node goes through `pii-channels`; never call the Discord API from the node.
- A post is **outward-facing**: never post a secret, a token, a path outside the workspace, or raw trajectory
  content; the node's input must be templated and bounded, and the guide says what may be posted.
- Never claim a live Discord post in the gate unless a test really made one (R5).
- Do not create forum tags from code; that is the human step in R4.
- Do not edit `specs/` except the verification section your orchestrator adds. Do not commit.
- Scratch under `/projects/_tmp/`, delete only your own, no daemon left running.

## Orchestrator verification (2026-10-10)

**My controls:**
- `cargo test -p pii-channels --lib post` → **8 passed** · `-p pii-server --lib post` → **8 passed** ·
  `--test post_e2e` → **5 passed, 1 ignored** (the ignored one is the live-Discord post, which needs a token).
- **MUTATION of my choosing, on the property that protects the user's phone:** releasing the idempotence key on
  a **timeout** (`crates/pii-channels/src/post.rs`, the timeout arm) reddens
  `a_timeout_is_bounded_and_keeps_the_key_because_the_send_may_still_land` with
  `left: Err(Timeout { after_ms: 100 })` vs `right: Ok(Duplicate)` — i.e. the retry posts a second time.
  Restored byte-identical → 8 pass.
- **LIVE end-to-end through a real daemon** (the part the worker could not run: it may not start background
  processes): a scratch workspace with one `post` node, a `[[channel]]` of kind `cli`, then
  `pii run start post-cli` →
  `{"run_id":"r000000000001"}` · the channel's outbox really contains
  `{"text":"hello from the post node","to":"out"}` · `pii metrics` renders **`pii_channel_posts_total 1`** and
  `pii_channel_post_refusals_total 0` · the daemon log carries `channel.posted`.
- The same live run also demonstrated the **fail-closed** direction twice, unprompted: a graph without
  `fs_write` → `channel.post_refused: capability denied: fs_write`; an outbox outside the channel's scope
  (`<runtime_dir>/channels/<name>`) → refused. Both are counted, not silent.
- The four repo checks: `deps-check`, `net-check`, `doc-drift-check`, `lint-observability` → rc 0 each.

**Worker's own deviations, accepted after checking:** the `post` capability is the coarse bit, not
`channel:<name>` (per-channel scope needs a `CapSet` field — filed as a follow-up); `to` must be an existing
thread/text channel, so a graph cannot open a new forum post (that remains the Hermes adapter's job); the
digest's inputs are limited to what a node can read today (`recall by_kind`), stated in its header comment.
The router's new "never send an empty reply" rule (`daemon/mod.rs:463`) is correct and documented in place:
a graph that already answered through `post` ends with `text = ""`, and Discord refuses empty content.

**Honest limits:** the classify node the worker added (outside the spec, because no node could route on
keywords) is deterministic rules only — no model; the live Discord path is `#[ignore]`d; `sys.status` counters
were verified in-process by the worker and by me through the CLI against a real daemon.

## Closure (2026-10-10)

**Committed** `dd5e0c9` (impl) after `e0007a6` (spec). Plain `fresh-gate` on the tree that carries it:
`quality-gate: ALL PASS · fresh-gate: PASS` (14 steps, `fed5e37`'s diagnostic + the two hygiene fixes below).

**Verified by the orchestrator, not taken from the worker's report:**
- `cargo test -p pii-channels --lib post` 8 · `-p pii-server --lib post` 8 · `--test post_e2e` 5 passed / 1 ignored.
- **My mutation:** releasing the idempotence key on a timeout (instead of keeping it) reddens
  `a_timeout_is_bounded_and_keeps_the_key_because_the_send_may_still_land` — `left Err(Timeout { after_ms: 100 })`
  vs `right Ok(Duplicate)`. Restored byte-identical. The property that protects the phone (a retry after a
  timeout must NOT post twice) is real and tested.
- **Live, with a real daemon** (the worker could not): `pii run start post-cli` → `{"run_id":"r000000000001"}`;
  the channel really sent — outbox `{"text":"hello from the post node","to":"out"}`; `pii metrics` rendered
  `pii_channel_posts_total 1 · pii_channel_post_refusals_total 0`; daemon log `channel.posted`. The same run
  showed the refusal path twice, fail-closed: no `fs_write` on the run → `capability denied: fs_write`; an
  outbox outside the channel's scope (`<runtime_dir>/channels/<name>`) → refused.
- Four repo checks (`deps-check`, `net-check`, `doc-drift-check`, `lint-observability`) at rc 0.

**Two gate blockers, neither of them this spec's logic — both fixed:**
- `xtask/tests/spec72_reachability.rs` pinned `crates/pii-server/src/lib.rs` as the file holding the
  `resolve_with_transport` caller. This spec's worker moved that call into `observed_provider.rs` (the 450-line
  split) and reported "I did not edit xtask", not noticing it had broken three reachability tests
  (`mutation failed: … not found in …/lib.rs`). The test now resolves the target file at test time, keeping the
  claim and dropping the path (`489efe5`, `8cf4eb1`; DO-NOT-REPEAT 1.47).
- `daemon::tests::a_quarantined_graph_is_refused_by_the_engine_and_runs_again_after_revert` (spec 125) failed in
  two gate runs and passed in four clean full-suite rounds (repo + export ×2). **The cause is NOT established**:
  the gate reported only test names until `fed5e37` made it keep the failing output at
  `<export>/gate-output.log` and echo the failing block. Its ceiling was raised 2 s → 60 s with the assumption
  stated (`5cb67a9`), but that was not the mechanism (it failed again at 60 s). Open item, now diagnosable on
  its next occurrence.

