# The Discord surface (deployment note)

Measured 2026-10-10 on the live guild. This file records what the *deployment* has, not what the code
can do — the code paths are described in `writing-a-channel.md` (spec 112) and `discord-graphs.md` (spec 128).

## The forum

| fact | value |
|---|---|
| forum channel | `pii` = `1558589728671146105` (type FORUM) |
| bot | `bella` = `1484212256437698612` |
| guild | `Bella` = `1484208204627054704` |
| previously configured home channel | `hermes` = `1539459188135305246` (a text channel) |

## Delivery: proven, and it posts as a THREAD

A Hermes delivery aimed at the forum id goes through the adapter's forum branch (`_send_to_forum`), which
creates a **post** whose starter message is the content — not a plain channel message. Evidence: a one-shot
`no_agent` job with `deliver=discord:<forum id>` produced post `1558591788615475311` containing
`"Hermes gateway -> pii forum: delivered by the cron delivery path..."`. A conversation that stays inside a
post is keyed as its own session (`chat_type="thread"`), so replies in a post do not bleed into other posts.

## Tag vocabulary

`chat` (default) · `task` · `spec` · `status` · `alert` · `done` — one tag per post is enough to triage from a
phone. Discord allows 20 tags per forum and 20 characters per name.

**The bot cannot create tags** (`PATCH /channels/<forum>` with `available_tags` returns `50013 Missing
Permissions`) — the set is a human action in the app, or the bot needs **Manage Channels** on that channel.

## The constraint that bites

The adapter creates forum posts **without `applied_tags`**, so a forum with **"Require tag"** enabled would
reject every Hermes delivery. Leave tags optional until either the tags are applied by the adapter or the
posts are created by a client that sets one.
