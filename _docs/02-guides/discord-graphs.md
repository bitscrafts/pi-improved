# Discord graphs: the `post` node and the first two graphs (spec 128)

Until this spec a graph could *receive* a channel message (the ingress starts a run) but could not *write*
into a channel except through the router's automatic end-of-run reply. The `post` node is the egress. It goes
through `Channel::send` of the daemon's configured channel — there is one Discord client (`pii-channels`),
and the node never calls the Discord API itself.

## The `post` node

```toml
[[node]]
id = "reply"
kind = "post"
caps = ["post"]
channel = "discord"                 # a [[channel]] name
to = "$.input.chat"                 # a channel/thread id (literal or $.expression)
template = "[${payload.class}] ${payload.text}"   # or: text = "..." / "$.path"
key = "triage-${run}"               # optional idempotence key
timeout_ms = 5000                   # default 5000, max 30000
max_bytes = 1900                    # default 1900, max 2000; longer text is cut and says so
```

| key | meaning |
|---|---|
| `channel` | required; must be a configured channel, else the node fails `post: channel '<n>' is not configured` |
| `to` | required; the destination. For Discord: a thread or text-channel id (see *Forum posts* below) |
| `text` / `template` | exactly one. `template` substitutes `${path}`: `input.*`, `payload.*`, `nodes.<id>.*`, and `run` |
| `key` | optional; may use `${…}`; at most 200 bytes |

The node returns on `ok` the value `{channel, to, bytes, duplicate, truncated, key?}` — the journal entry of
the node therefore carries the destination, so a run can be traced to what someone saw on their phone.
There is **no `err` port**: a refusal or provider failure fails the node, loudly, with the channel's own
message (bounded to 600 bytes). Failures that are transient (`unavailable`, `deadline exceeded`) are retried
by the engine when the node has `retry = { max = N }`.

### Capability

The graph **and** the node must hold `post` (a node's caps default to the kind's; `caps = []` removes it). A
graph that omits it fails validation (`V2: node 'p' requires capability 'post'`); a node without it fails the
run with `capability denied: post: channel <name> not granted (the graph and the node need the post
capability)`. This is the coarse `post` bit — it is **not** per channel: any channel name configured in the
daemon is reachable by a graph that holds `post`. A per-channel scope (`channel:<name>`) is not built; that
needs a `CapSet` field and is filed as a follow-up, not done here.

### Idempotence (R2)

A retried node must not double-post. With `key`, the hub remembers `(channel, to, key)` **in memory, per
daemon** (bounded: the 4096 most recent keys; a restart forgets them). The mechanism:

- the key is reserved *before* the send; a second post with the same key returns `duplicate: true` and sends nothing;
- a send that **fails definitively** releases its key, so the retry can post (nothing was delivered);
- a send that **times out keeps** its key: the stalled send may still land, so the retry is suppressed
  (a lost post is better than a duplicate in someone's phone).

`key = "…-${run}"` makes the key per run: a retry inside one run posts once, a new run posts again. Without a
`key` there is no protection. Proven in `crates/pii-server/tests/post_e2e.rs` (engine retry, stall + retry)
and `crates/pii-channels/src/post.rs`.

### Bounds

`timeout_ms` bounds the call (the send runs on its own thread; the node stops waiting after `timeout_ms` and
fails with `deadline exceeded after <n>ms`) — a hung channel never hangs a run. Text is cut at `max_bytes`; an
error message from the provider is cut at 600 bytes with its start intact.

### What may be posted

A post is outward-facing: it ends up in a person's phone and in a third party's logs. **Never post** a
secret or token, a path outside the workspace, raw trajectory content, or an unbounded model output. Template
only fields you chose (`${payload.class}`), not `${input}` wholesale. The triage graph echoes the sender's own
message back to the same thread — that is the one deliberate echo.

## The loop: ingress → run → post

`[[channel]] kind = "discord"` polls the channel; each message starts the channel's `graph` with the input
`{text, chat, user, channel}`; `chat` is the id of the channel (or thread) the bot polls. The router also
replies with the run's final output **unless it is empty** — a graph that already answered through `post`
ends in a node whose output is `{text = ""}` so nothing is sent twice (this empty-reply rule is new in this spec).

## The two graphs

### `graphs/discord-triage.toml`

`classify` (a new pure node: ordered `prefix`/`suffix`/`any` rules, case-insensitive, first match wins, no
model) sorts the text into `alert | done | spec | status | question`, and `chat` when nothing matched. `reply`
posts `[class] text` into `$.input.chat` — the same thread. `done` records the outcome.

Journal of one run (measured in `post_e2e.rs`, cli loopback standing in for Discord):

```
classify -> {"class":"question","matched":"?","text":"is the build green?"}
reply    -> {"bytes":30,"channel":"discord","duplicate":false,"key":"triage-r000000000001","to":"thread-111","truncated":false}
done     -> {"outcome":"triaged","text":""}
event channel.posted: channel=discord destination=thread-111 bytes=30
```

### `graphs/discord-digest.toml`

Posts `pii digest: N device(s) known, M lesson(s) remembered.` into `$.input.to`. **Inputs, stated:** two
`recall by_kind` reads of the entity store (`device`, `lesson`; spec 126/127, ranked). **Not inputs:**
`pii learn list` candidates, memory tier counts and recent run outcomes — no graph node reads them today
(they are daemon status); a node for them is a separate spec. To run it daily, front it with a `timer`
node and carry `to` in the `[[cron]]` section's `input`.

## Forum posts and tags — the human step (R4)

`to` is an **existing** thread or text channel: `Channel::send` posts a message (`POST /channels/<to>/messages`).
*Creating a new forum post* (a thread whose starter message is the content) is what the Hermes adapter's
`_send_to_forum` does; the `post` node does not. To post a new forum thread from a graph the adapter (or a new
channel method) must change — out of scope here, filed.

**Tags are a human step.** Measured 2026-10-10: the bot's `PATCH /channels/<forum>` with `available_tags` returns
`50013 Missing Permissions` (see `discord-surface.md`). In the Discord app, forum `pii` → *Edit channel* → *Tags*,
add: `chat` · `status` · `spec` · `alert` · `done` (emoji optional; `task` also exists in the deployment). They
exist now with these ids: `chat`=1558593520950976588 · `status`=1558593520950976591 · `spec`=1558593520950976590 ·
`alert`=1558593520963690607 · `done`=1558593520963690608. If the bot should one day *apply* a tag when it
creates a post, that is an adapter change (`applied_tags`), not a graph change — file it, do not smuggle it in.

## Observability

Events `channel.posted` (`channel`, `destination`, `bytes`) and `channel.post_refused` (`channel`, `reason`),
both in `names::ALL`. `pii metrics` prints `pii_channel_posts_total` and `pii_channel_post_refusals_total`
(from `sys.status` `counts.channel_posts` / `channel_post_refusals`).

## Testing

No live Discord in the gate: `post_e2e.rs` uses the `cli` channel under the name `discord` plus an in-test
flaky channel. The live path is `live_discord_post` (`#[ignore]`d; needs `PII_DISCORD_TOKEN`,
`PII_DISCORD_CHANNEL_ID`, `PII_DISCORD_TEST_THREAD`): `cargo test -p pii-server --test post_e2e -- --ignored`.
