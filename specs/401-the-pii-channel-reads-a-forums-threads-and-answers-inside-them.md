# Spec: the pii channel reads a forum's threads and answers inside them

## Overview
The `pii` Discord channel is a FORUM (type 15). pii polls `GET /channels/{forum}/messages`, and a forum has no messages of its own, so
the poll returns `[]` with HTTP 200 forever (measured 2026-11-09 on the production host: 0 messages; the forum's 2 active threads hold 6
messages, all from the bot, none from a person). A human reply can therefore never reach pii: the channel is one-way by construction, and
the empty poll hides it. Replies live in `GET /channels/{thread}/messages`.

Three more facts decide the shape of the fix:
- A reply goes to `Incoming.chat`, and a send to a forum id creates a NEW thread, so an answer must go to the THREAD id.
- The bot token is the same as the Hermes gateway's, and the channel does not drop its own messages. Once threads are read, the first
  answer would be read back and answered again without end (`discord-triage` replies for every class, "chat" included).
- Anyone in the guild can post in a thread, so who may talk to pii is a decision of the committed config, never of the message.

**Project**: /projects/pi-improved
**Phase**: Discord two-way (follows 132, forum posting, and 333, poll pacing)
**Depends on**: 132

## Requirements
- R1 When the configured channel is a forum (the shape probe of spec 132, already cached), `poll` lists the guild's active threads
  (`GET {base}/guilds/{guild_id}/threads/active`, the guild id taken from the probe), keeps the threads whose `parent_id` is the forum,
  skips a thread whose `last_message_id` is not greater than the cursor, and reads `GET {base}/channels/{thread}/messages?after={cursor}&limit=20`
  for each remaining one. The cursor stays ONE number, the highest message id returned (ids are time-ordered across channels). The result is
  oldest first, and `Incoming.chat` is the THREAD id.
- R2 An idle poll costs one request: when no thread is newer than the cursor, no message request is made.
- R3 Loop guard. The channel learns its own bot id once (`GET {base}/users/@me`, cached) and never returns a message that bot wrote. A message
  from any other bot is not returned either (bots are context, never requests). If the id cannot be learned the poll is refused, never run
  unfiltered.
- R4 Allow list. A message is returned only when its author id is on the allow list: the comma-separated ids in `PII_DISCORD_ALLOW_USERS`,
  else the guild owner (`GET {base}/guilds/{guild_id}`, cached). Any other author is dropped and counted; the dropped text is never logged.
  If the list cannot be resolved the poll is refused (fail closed).
- R5 `send(ctx, to, text)` with `to` a thread id posts one plain message to `/channels/{thread}/messages` and never creates a thread.
- R6 A non-2xx answer from the thread listing or a message read is an `Unavailable` error that names the endpoint, not an empty poll.
- R7 A text channel polls exactly as before.
- R8 `_docs/05-operations/discord-topics.md` says what the channel reads, who it answers, and that it reads the forum's active threads only.

## Architecture
```
human posts in a thread of the forum
  poll: probe(forum) -> guilds/{g}/threads/active -> keep parent==forum, last_message_id > cursor
        -> channels/{thread}/messages?after=cursor -> drop own bot, other bots, authors not on the allow list
        -> Incoming { chat: thread id, user, text, cursor }      (one run per message, as today)
  reply: send(thread id) -> /channels/{thread}/messages           (a thread is not a forum: shape Text)
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `a_forum_poll_reads_the_active_threads_oldest_first` | forum probe, 2 active threads with messages 30, 10 and 20 | messages 10, 20, 30; `chat` is each message's thread id |
| `an_idle_forum_poll_makes_one_request_after_the_probe` | every thread's `last_message_id` <= cursor | no `/messages` request |
| `a_message_from_our_own_bot_is_never_returned` | a thread holding one bot-authored and one listed-human message | only the human's |
| `CONTROL_a_listed_human_in_the_same_thread_is_returned` | the same thread, human message newer | returned, with the right `chat` |
| `a_message_from_another_bot_is_not_returned` | author `bot: true`, another id | not returned |
| `an_author_not_on_the_allow_list_is_dropped_and_counted` | unlisted human | not returned; the drop counter is 1; no text in any log line |
| `the_default_allow_list_is_the_guild_owner` | no env, owner id 7 | author 7 returned, author 8 dropped |
| `the_env_allow_list_replaces_the_default` | `PII_DISCORD_ALLOW_USERS=8` | author 8 returned, owner 7 dropped |
| `an_unresolvable_identity_refuses_the_poll` | `/users/@me` answers 500 | `Err`, never an unfiltered list |
| `a_reply_to_a_thread_id_posts_a_message_in_that_thread` | `send(.., thread_id, ..)` | one POST to `/channels/{thread}/messages`, none to `/threads` |
| `a_failed_thread_listing_is_an_error_not_an_empty_poll` | `/threads/active` answers 403 | `Err` naming the endpoint |
| `CONTROL_a_text_channel_polls_as_before` | channel type 0 | the same single `/messages` URL as today |

Acceptance test: `a_message_from_our_own_bot_is_never_returned`. The requirement a plausible implementation satisfies in appearance only is
R3: a filter on `author.username` passes a test whose fixture shares a name, and fails live where the bot and a person differ by id. The fixture
uses the live shapes (message `{id, content, author:{id, username, bot}}`; thread `{id, parent_id, last_message_id, type}`, ids as strings) and
the two authors have DIFFERENT ids and the SAME username.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that passes today is a Guardrail, never a criterion here.
- [ ] `cargo test -q -p pii-channels --test discord_forum_inbound 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `test -f crates/pii-channels/src/discord/inbound.rs`
- [ ] `grep -q "reads the forum's active threads" _docs/05-operations/discord-topics.md`

## Guardrails
- Keep these green; they pass today, so they are not criteria: `cargo test -q -p pii-channels --test discord` and `cargo test -q -p pii-channels --lib`.
- Do NOT change `Incoming` (every channel builds one): the guard and the allow list run inside the Discord channel, before anything is returned.
- Do NOT put the new code in `discord.rs` (443 lines, the effective-line limit is 450): it goes in `discord/inbound.rs`.
- Do NOT log or count message text. Do NOT widen the egress hosts (`discord.com` only). Do NOT poll archived threads.
- Do NOT change the forum send path of spec 132 (a send to a forum id still creates a thread).
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `crates/pii-channels/src/discord.rs`
- `crates/pii-channels/src/discord/inbound.rs`
- `crates/pii-channels/src/discord/forum.rs`
- `crates/pii-channels/tests/discord_forum_inbound.rs`
- `crates/pii-channels/tests/discord.rs`
- `_docs/05-operations/discord-topics.md`
