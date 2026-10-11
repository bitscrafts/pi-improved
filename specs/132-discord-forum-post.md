# Spec: 132 — the Discord channel can post into a forum, because a forum needs a tag

**Project**: /projects/pi-improved
**Phase**: after 128 (the `post` node) and 130. Filed 2026-10-11 by the orchestrator.

## Overview

The user's `pii` surface **is a Discord forum** (`1558589728671146105`), and it has `REQUIRE_TAG` set: Discord
rejects a forum post that carries no tag. Measured today on the Hermes side, where the same wall appeared:
`{"message": "A tag is required to create a forum post in this channel", "code": 40067}`.

**Measured live, 2026-10-11, against the user's forum (nothing was created — both calls fail):**

| path | result |
| --- | --- |
| `POST /channels/<forum>/messages` (what pii's `send` does today) | **400 `50008` "Cannot send messages in a non-text channel"** |
| `POST /channels/<forum>/threads` with no tag | **400 `40067` "A tag is required to create a forum post in this channel"** |

So the gap is worse than a missing tag: a forum cannot be messaged at all — pii's current path fails with
`50008`, and the correct path additionally needs a tag.

pii's Discord channel posts a **plain message** and cannot cross that wall:

```rust
// crates/pii-channels/src/discord.rs:135-140
fn send(&self, ctx: &Ctx, to: &str, text: &str) -> Result<(), PiiError> {
    let body = serde_json::to_vec(&serde_json::json!({ "content": text }))
        ...
        url: format!("{}/channels/{to}/messages", self.base),
```

A forum accepts no messages at all: a post is a **thread** (`POST /channels/{forum}/threads`) carrying
`applied_tags`. So every graph that posts into the user's forum — the point of spec 128 — fails there, and the
failure would be the provider's 40067 rather than a named, actionable refusal.

Hermes' adapter already proved the working shape (thread create + `applied_tags`, tag resolved by name), and
spec 128 gave the `post` node a channel + a bounded call + an idempotence key. This spec closes the last gap
between them.

## Requirements

### Functional

- **R1 — the target's shape is known before the post.** The channel distinguishes a **forum** (Discord type 15)
  from a **text channel** by asking Discord (`GET /channels/{id}`), not by config alone; the answer is cached on
  the channel instance with the TTL stated in the code, and a failed probe is a refusal, not a plain-message
  fallback (posting a message into a forum is the failure this spec removes).
- **R2 — a text channel is unchanged.** The existing plain-message path keeps its behaviour, its error handling
  and its tests (including 128's idempotence).
- **R3 — a forum post is a thread with a tag.** For a forum, the post creates a thread carrying `applied_tags`
  with the tag resolved **by name**: the per-post tag if the caller supplied one, else the channel's configured
  `forum_tag` (default `chat`). Name, not id — the user renames tags on the phone and ids are opaque.
- **R4 — no tag, no silent failure.** If the forum requires a tag and no name resolves to an existing tag, the
  send is **refused** with a message naming the forum, the tag names it tried, and the available tag names; the
  provider's own error is preserved when Discord is the one refusing (bounded, scrubbed, as the existing tests
  require).
- **R5 — a retry never double-posts.** 128's idempotence key covers the thread path too: two sends with the same
  key create one thread.
- **R6 — the `post` node can carry a tag.** The node's optional `tag` reaches the channel on the forum path
  (natural increment of 128: a digest posts `status`, a failure `alert`), and is ignored — not an error — on a
  text channel.
- **R7 — bounds.** Thread name ≤ 100 chars (derived from the text's first line, with the rule stated), content
  ≤ 2000, the channel's existing timeout applied to both calls.

### Non-Functional

- No new dependency (the existing HTTP seam in `pii-channels`); 450-line file rule; `cargo fmt` +
  `clippy -D warnings` clean; tests create their scratch; the mock HTTP transport the existing channel tests
  already use is the test vehicle — plus ONE live test marked `#[ignore]` that posts into a real forum when
  `PII_DISCORD_TOKEN`/`PII_DISCORD_FORUM` are set.

## TDD Contract

| Test Name | Input | Expected Output |
| --- | --- | --- |
| `a_text_channel_still_posts_a_plain_message` | channel type 0 | one POST to `/channels/{id}/messages` (unchanged path) |
| `a_forum_post_creates_a_thread_with_the_configured_tag` | channel type 15, no per-post tag, `forum_tag = "status"` | one POST to `/channels/{id}/threads` whose `applied_tags` is the id of the tag named `status` |
| `a_forum_post_uses_the_per_post_tag` | type 15, per-post tag `alert` | `applied_tags` is the id of `alert`, not the default |
| `a_forum_with_no_resolvable_tag_is_refused_and_counts` | type 15, `forum_tag = "nope"`, available tags `[chat, status]` | `Err` naming the forum, the tried name and the available names; the refusal counter increments |
| `the_providers_refusal_is_preserved_and_scrubbed` | forum create returns 400 + `40067` + a token-shaped string | the error carries the provider's message, scrubbed and bounded |
| `two_sends_with_one_key_create_one_thread` | the same idempotence key twice | one thread created; the second call reports duplicate |
| `the_post_node_forwards_its_tag` | a `post` node with `tag = "status"` on a forum | the thread carries `status` |

## Exit Criteria

- [ ] `cargo test -p pii-channels` passes with the seven tests above present and passing
- [ ] `cargo test -p pii-server --lib` passes (the node forwards the tag; 128's tests keep their claims)
- [ ] mutation: making the forum path fall back to a plain message reddens
      `a_forum_post_creates_a_thread_with_the_configured_tag` (output pasted, restored byte-identical)
- [ ] `cargo run -q -p xtask -- net-check` and `deps-check` exit 0, and the new host is inside the channel's
      existing egress scope (no new allowlist entry)
- [ ] `bash quality-gate.sh` passes in the dev tree
- [ ] **live — the ORCHESTRATOR does this, with the token read at runtime and never printed:** the worker
      implements and proves the paths with the mock transport, and must NOT go looking for a token or take a
      dependency on one. Say plainly in your report that the live proof is mine to run.

## Guardrails

- Do NOT post a plain message into a forum "and see" — that is the bug.
- Do NOT hardcode a tag **id**: names are what the user edits on the phone.
- Do NOT cache the channel-type probe without a stated TTL, and do NOT cache a *failed* probe as "text".
- Do NOT break 128's idempotence: the thread path must honour the same key.
- Do NOT add a dependency for HTTP or JSON; the seam and `serde_json` are already there.
- Do NOT touch `specs/`.

## Closure (2026-10-11) — the live proof failed three times, each for a real pre-existing reason

The mock tests were green from the first implementation. The live run against the user's forum failed three
times, and every failure was a defect the mocks could not see:

1. **`api.discord.com` does not resolve.** `DEFAULT_API_BASE` pointed at a host that does not exist (since a
   phase-3 checkpoint), so neither `send` nor `poll` could ever reach Discord. Fixed to
   `https://discord.com/api/v10` and **pinned by a test** (`the_default_base_is_the_real_discord_api`).
2. **The channel's egress grant was never read by the network layer.** `channel_ctx` set `Caps.net_hosts`, and
   the unit tests asserted it — but the dial-time decision comes from **`pii-net`'s own grant store**
   (`ConfigGrants`, built from `[net] grants`), which had nothing for the channel principal. So the caps were a
   value nothing on the live path consulted: `capability denied: net: target 'discord.com:443' not in grant scope`
   with every test green. The daemon now registers the channel's declared hosts for that principal
   (`ConfigGrants::allow_declared`), and a principal that declares nothing still gets exactly the config grant —
   **fail-closed**.
3. The graph's grant is untouched throughout: the graph grants `post` (intent); the **channel principal** carries
   the egress scope. That separation is what stops a graph from gaining the network.

**Live proof (orchestrator, real forum, token read at runtime and never printed):**

```
LIVE run r000000000001  status succeeded
node p finished: {"bytes":28,"channel":"discord","to":"1558589728671146105","duplicate":false}
NEW THREAD 1558696818995372134 | tags: ["1558593520950976588"]   ← the `chat` tag
  content: "pii spec 132 live forum post"
delete http=200   ← the test thread removed; nothing left on the phone
```

**My mutations (independent of the worker's):**
- a failed shape probe cached as `Text` → `a_failed_probe_is_a_refusal_not_a_plain_message_and_is_not_cached`
  reddens (`left: Post · right: Get`) — the 50008 bug it prevents;
- `allow_declared` registering nothing → `a_channel_may_dial_the_host_it_declares_and_no_other` reddens
  ("the declared host is granted"). Both restored byte-identical.

**Verified:** `pii-channels` 13 + 12 + 2 · `pii-server --lib` 117 · `post_e2e` 6 passed / 2 ignored · four repo
checks rc 0 (`deps`, `net`, `doc-drift`, `lint-observability` 0 findings).

**Limits, stated:** the worker's real-path grant test uses a **telegram** channel (a Discord channel needs token
env vars), so the Discord host specifically rests on the unit-tested `egress_hosts` plus the live run above;
`channel_ctx` still sets the hosts on the `Ctx` caps even though the dial check reads the grant store — kept
because it is the correct shape for that layer, and named here rather than left implicit.

