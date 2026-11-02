# Discord topics — what belongs under which tag

The `pii` channel is a **forum** (type 15), and a forum's post carries exactly one **tag**. Those tags are the
`topics`: they are what makes the surface readable instead of a flat list of everything ever said.

Measured 2026-11-02 (`GET /channels/1558589728671146105`): the forum carries six tags —

```
chat    conversation: a question, an answer, anything a person would reply to
task    a unit of work in flight, named by what it is doing
spec    a specification authored, or a spec whose criteria were closed
status  a periodic or lifecycle statement: digests, health, "the daemon came up"
alert   something went wrong and needs a person: incidents, refusals, failed runs
done    a run finished, a row closed, a deploy landed
```

## Who sets what

| producer | tag | why |
|---|---|---|
| `graphs/discord-digest.toml` → `post` node | **status** | the daily/summary line is a statement of state, not a conversation |
| triage reply (a message answered in the forum) | **chat** (default) | it is the reply to a person |
| `pii` startup notice | **none — silent** | opt-in via `PII_VERIFY_POST=1`; a forum turns every message into a thread, so a restart must not create one (spec 332) |
| incidents (`_tmp/autonomy/INCIDENTS.md`, cron `cf97eea12eba`) | **alert** when posted through pii | an incident is the one thing that must not be missed |

`PII_DISCORD_FORUM_TAG` names the default for a post that does not say: it is `chat`. A graph overrides it with
`tag = "<name>"` on the `post` node. The adapter resolves the tag **by name** against the channel's current tags
(a probe, cached for `SHAPE_TTL`, because tags get renamed on the phone), so a post never invents one — an unknown
name is a refusal, not a silent untagged post.

## Rules

- One tag per post. If a message wants two topics, it is two messages.
- The tag is part of the post, not decoration: `alert` and `done` are read as a queue.
- Never tag an automated post `chat` — that is the human's topic, and a bot there buries the conversation.
