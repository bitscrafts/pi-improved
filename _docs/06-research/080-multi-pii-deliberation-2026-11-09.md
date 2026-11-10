# Several pii agents discuss a GitHub item and decide the best approach

Filed 2026-11-09. Design note, nothing built. It corrects the earlier evaluation (079), which sketched ONE agent classifying repo items.
The goal as the operator states it: **multiple pii's discuss a GitHub item and decide the best approach**, with the pii Discord channel as
the place a person is told and answers.

## 1. What the workspace memory holds (searched five ways on 2026-11-09)

Nothing on "several pii's deliberating on a GitHub item". Neighbours, each one idea:

- EGM (`research/elephant-goldfish-model-egm`): an agent with the project context drafts, a reviewer with ZERO context reviews, the human resolves
  trade-offs. A review that cannot get a fresh context STOPS, it does not review in-context. Same-context agents agree with themselves.
- pidag spec-37/38: a critic's verdict is a leading PASS/FAIL token, unparseable fails closed; a quorum node counts recorded verdicts and is wired
  with `after`, never `depends_on`, because it must still run when critics disagree.
- `research/agents-loops-graphs-framework`: a loop needs a check that can fail and a hard attempt limit; the agent that did the work never checks it.
- LangChain company harness: two identities per agent (on behalf of the asker, or its own), a whitelist for who may make it post.
- dsh-superagent (079): bots produce context and never requests; observe by default; stop by impact with one question.

## 2. The design

Medium: the GitHub item itself. Each pii is an independent daemon (the production host, the development daemon, or another host) with its own model
and role. They share no process and need no federation: the issue/PR thread is the blackboard, comments are the messages. This keeps "one daemon
per host" true.

```
roles (one literal per graph, each its own model, none sees another's reasoning, only the thread):
  proposer   reads the item and the repo, posts ONE approach with the evidence it read
  critic     zero context beyond the thread and the files it cites; posts the weakest point of each approach
  arbiter    does not propose; counts verdicts, applies the rule, posts the decision or the single open question
```

Protocol, bounded so it cannot run away:
1. `repo-watch` opens a round for a new item (queue row keyed by item id, so one round per item).
2. Round r: each proposer comments once; each critic comments once. A comment carries a machine header (`pii-round: r`, `role:`, `verdict:`).
3. Stop when the arbiter sees quorum (for example 2 of 3 critics PASS the same approach), or after N rounds (N is a literal, 3).
4. No quorum at N: the arbiter posts ONE concrete question to the Discord pii channel and the item waits for the person. It never decides alone.
5. A decision is a comment on the item plus one Discord line. Acting on it (a branch, a PR) is the existing `queue` and `spec` lane, in observe mode first.

Safety, from rows 399/400 and the dsh authorization model:
- Which authors count as a pii is a committed allow list of GitHub logins. A comment from anyone else is CONTEXT and never a vote or an order.
- A comment cannot widen anything: text is data. Programs, caps and the allow list are literals in the graph file.
- Each pii verifies a peer's header against the allow list before counting it; a forged header from a non-listed login is ignored.
- Attempt limit and cost ceiling per item; a round that exceeds either ends as "stopped, with one question".

## 3. Why the Discord channel comes first

The decision has to reach the person, and the person has to answer. Measured 2026-11-09 (counts only):
- pii polls `GET /channels/{forum}/messages`. The `pii` channel is a FORUM (type 15). A forum has no messages of its own: that URL returns `[]`
  (HTTP 200). Replies live in `/channels/{thread}/messages`. So pii can never read a human reply: the channel is one-way by construction.
- The forum holds 2 active threads and 6 messages, all from the bot, none from a human. Nothing archived.
- Even if a reply were read, `reply()` sends to the forum id, and a post to a forum creates a NEW thread, so an answer would not land in the
  conversation it answers.
- The bot token is the same as the Hermes gateway's (the workspace memory `pii/2026-11-02/discord-429-is-a-shared-bot-token`), and pii does not drop its own
  messages. The moment it reads threads it would read its own post and answer it, forever: `discord-triage` replies to every class, "chat" included.
- 731 of 732 logged poll failures are 429s from 11-02 02:00-03:59, before the pacing fix; none since, including after today's deploys.

So the order is: (1) read the forum's threads, with the self-message guard and an allow list; (2) answer inside the thread; (3) then the
deliberation graphs, which post to the item and, when they need a person, to this channel.

## 4. Measurements before building the deliberation

1. A thread of two pii comments round-trips: `gh api` reads a header the other wrote, and the allow list rejects a forged one.
2. Three roles on the local model give a verdict token that parses; an unparseable reply fails closed. Reuse the judge harness (spec 144), 30 labelled items.
3. A round ends at the attempt limit with one question and no further model call (control: below the limit it continues).
4. Cost per round is recorded and the ceiling stops a loop (control: a loop under the ceiling completes).

## 5. Addendum (2026-11-09, decisions taken in discussion with the operator)

1. **The spec stays a FILE** under `specs/NNN-*.md`, committed on the PR branch. The PR body only LINKS it at a commit sha (a permalink cannot change).
   A body copy was rejected: it can be edited after review, has no line review, has a size limit and creates a second source of truth.
   A PR with no diff yet opens with an empty first commit.
2. **Approval is `approve <spec-commit-sha>`** from the owner's allow-listed login. The lane runs a spec's criteria only for the approved commit; a later
   edit is a new commit and needs a new approval. A verdict cites the commit it reviewed, so a verdict on an old version never counts for a newer one.
3. **Templates for everything** (drafted under `<scratch>/pii-templates/`): a PR template, an issue template and five comment templates
   (proposal, critique, implementation-ready, decision, question). A comment that does not match its template is ignored and counted. Free model text never decides.
4. **A unique id per installation**: random 128 bits generated at first start, stored in the STATE directory (never the config, so a copied config cannot copy an
   identity), shown as a short form. An id proves nothing alone: the vote counts only when the id matches the roster entry of the comment's GitHub AUTHOR login.
   Two installs claiming one id both refuse to vote and ask the human. To confirm: the deploy backup and rollback must keep the id file, or a rollback
   silently changes the host's identity. Instance names must not contain a tool or assistant name.
5. **The roster** is a committed `.pii/roster.toml`, changed only by a PR the owner merges.
6. **Discordance goes to a human.** Unanimity is the only thing that proceeds alone. Any dissent, abstention or silence by the deadline is one question to the
   owner on the PR and on the pii Discord channel (same template). The owner's decision, from the allow-listed login, is final for that round. Merge is always the owner's.
7. **Order**: the Discord two-way channel (row 401, done) comes first so pii can ask; the mechanism is proven on a throwaway sandbox repo (to be created only
   with the operator's go-ahead, an external action); only then does it act on a real repository.
8. **Spec review before queueing** (see note 081): the proposer/critic/arbiter loop is the same loop as the EGM review, with the PR as the medium.
