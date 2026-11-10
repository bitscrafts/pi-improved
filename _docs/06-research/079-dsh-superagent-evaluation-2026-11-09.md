# DeepSeek dsh-superagent: what qualifies for pii, and the set of graphs it suggests

Filed 2026-11-09. Evaluation only: nothing here is built or queued as a spec. Facts are tagged by how I got them.

## 0. Evidence and its limits

- READ in full or in large part: the repo README, `docs/002-authorization.md`, `docs/003-execution.md` (head and tail), `docs/007-architecture.md` (head), the offloop.org explainer, the workspace memory `research/dsh-superagent-deepseek` and `insights/dsh-superagent-deepseek` (stored today).
- NOT read: `000`, `004`-`006`, `008`, the middle of `001`/`002`/`003`, the original Chinese article, the code itself. Benchmark figures below come from a third-party summary.
- The repo says of itself: research prototype, not an official product, much of it written by coding agents, "some designs ... are NOT implemented here".
- The benchmark is synthetic: coding is replaced with simulated delays, five runs per configuration. It measured scheduling, not code quality. Reported: median completion about 39% lower than a neutral-prompt baseline and about 10% lower than a delegation-first baseline, at about 2.1x the cache-read tokens. I would not use these numbers for any decision.
- the workspace memory check (2026-11-09, five queries): two entries on this exact topic, both stored today (`research/dsh-superagent-deepseek`,
  `insights/dsh-superagent-deepseek`). Nothing newer. Related older entries: `research/deepseek-harness-qwen38` (the harness, not this
  agent; notes a small model looping for hours re-planning on an existing codebase) and `workspace/architecture/append-only-event-log-pattern`.
- Web check, additional facts (search snippets only; the SMF article and one long page could not be fetched, the extract backend hit its
  daily cap and the browser daemon did not start): the repo is ONE commit (6337fc8, 2026-10-31) and v0.1.0. Proactive work has THREE accept
  modes: `off`, `observe` (the default, records what it would have accepted) and `auto`. The authors state it runs with full OS-user access
  and should be isolated before use. Their own list of open problems: scaling the Router, long-term memory, agent-to-agent communication,
  and prompt injection when a cloud agent can reach local credentials. A third-party code review (SMF Clearinghouse, pinned SHA) reports the
  governance is real in the tree while the headline modules of the design article are missing; I have only its summary, not the page.
- Simulated results as reported by two outlets: p95 task time 15.7 min vs 29.5 min for a single-session baseline, median 12.4 min; without
  direct routing of follow-ups to workers, router decisions rose about 30%. Same caveat: simulated delays, not code.
- Star counts disagree between sources (5k, 20k+, 21.6k). Irrelevant to the design, noted so nobody quotes one.

## 1. What it is

A proactive coding agent for ONE repository. It listens to GitHub (and optionally Feishu). When an allowed person asks it for work, it opens a work item (a Ticket), works in its own session and directory, and answers where the request came from, e.g. with a pull request. A web panel shows only what needs a person. Written in TypeScript on DeepSeek Harness.

Its domain model has few concepts (docs/001): Source -> Trigger -> Thread -> Eval -> Ticket (with a Mandate) -> Assignment -> Executor, with a Channel as the write side and a Policy as the versioned rules.

Note on "observe": it is a default, not a setting the team must remember to choose. That is the part worth copying, and it is one
literal per graph in pii's terms.

## 2. What qualifies for pii

Mapped to what pii has today. "Have" means present in this tree; "Need" means I found nothing that does it.

```
dsh concept            pii equivalent                                   status
Source (event stream)  [[channel]] / [[cron]] / `timer` node            have; no git source yet
Trigger (immutable)    a keyed queue row (`queue.put`, `key=`)          have the primitive
Thread (grouping)      the key (branch / PR / issue id)                 convention, not built
Eval (start or not)    `judge` + `route`                                have both kinds
Policy (allow list)    committed graph literal / host config            have the principle (row 400)
Mandate (per-work caps) graph `caps` + ArgvScope + declared `programs`  have, stronger than theirs
Ticket (work item)     a run: journaled, resumable, effects sandwiched  have
Assignment/Executor    `coder` node via `call spec`                     have
Channel (write side)   `post` kind, `key` = exactly once                have (Discord live)
Report ready/blocked   exit ports + `gate` (suspend for a person)       have
Panel                  Digest on demand (`digest-now`)                  have
observe mode           none                                             NEED
stop by impact         none                                             NEED (a `judge` verdict + `route`)
worker slots / limit   unmeasured                                       NEED to measure
fan-out per trigger    "no iteration primitive" (COMPOSABILITY.md:110)  NEED, worked around by the queue
```

Three ideas are worth taking, in this order:

1. **The authorization rule: "messages are inputs to a decision and cannot change permissions."** This is the same principle as row 399/400 (authority comes from the committed file, never from run input). It gives the repo agent its security model for free: the allow list lives in the committed graph or host config, and the text of an issue or commit message is only ever data. This is the single most useful import.
2. **Default observe mode for unrequested work.** Proactive steps start by recording what they would have done, until the operator widens them. pii's autonomy loop acts today; this lets us prove intent before it spends or commits. It is also how I would roll out the repo agent itself.
3. **"Stops by impact, not effort," and one concrete question at a time.** A local change runs to a reviewable result; anything that needs a design decision stops with ONE question to a person. This is our escalation doctrine, stated better, and it maps to a `judge` verdict feeding a `route` into a `gate`.

What does NOT transfer, and why:

- **Their worker permissions.** Executor sessions run commands without approval as the operator's OS user (docs/003). pii's capability scopes, deny list and now declared `programs` are stricter. Do not adopt theirs.
- TypeScript, DeepSeek Harness presets, the cordis plugin system, Feishu, and the React panel: all specific to their stack. pii is one Rust binary, and the panel is Discord plus an on-demand digest.
- The eval retry machinery (two retries, then `unavailable`), pause processing, and the ticket-stage table: a lot of detail for a multi-person team. pii has one operator; take the idea of a stage table for the digest, not the engine.
- Multi-person admission tables. pii has one operator and a short allow list.

## 3. A set of graphs for a git-repo agent

Principle from the operator: policy in graphs, capability in the binary, no new node kinds without a measured need. Everything below uses the 14 existing kinds. The binary gains at most a `git` source op if the `tool` node cannot do it (it can: `tool = "run"`, `programs = ["git"]`, which is exactly what row 400 makes safe).

```
repo-watch    timer -> tool run git fetch + log since cursor -> route(none|new)
              -> ledger queue.put per new item, key = commit/branch id (exactly once) -> remember cursor
repo-eval     judge (kind: local | design | unclear, bounded schema, local llama.cpp)
              -> route: local -> enqueue ; design -> post ONE question + gate ; unclear -> record only
repo-do       the existing `queue` -> `spec` graphs (coder in a worktree, verify, docs) -- no new graph
repo-report   post (key = ticket) only when a person must act or can use a result
repo-mode     one literal per graph: observe | act. Observe replaces every effect with a `ledger` record
```

Why the queue carries the fan-out: pii has no iteration primitive (`graphs/COMPOSABILITY.md:110`), so a watch cannot loop over N new items in one run. It writes N rows, and the queue (already one run per row, journaled, resumable) drains them. That is the existing Trigger-to-Ticket path, and it needs no new kind.

Safety properties it would inherit, each already enforced:
- an allow list is a committed literal; a commit message or issue body cannot widen it
- `programs` are literal (row 400), so the repo's own content cannot choose what executes
- a worker never commits; the orchestrator does (existing rule)
- effects are journaled before and after, and an unconfirmed effect refuses to resume

Content, not only code: the same lane works for docs or books (the `books` graph already exists); the `judge` verdict and the `spec` graph are the only parts that differ.

## 4. Before building anything: four measurements

1. **Is a git source expressible today?** Run `tool run git` with a literal `programs = ["git"]` against a scratch repo, store the cursor with `remember`, and read it back. Pass or fail, one afternoon.
2. **Does a `judge` node give a stable three-way verdict on the local model?** Use 30 hand-labelled issue texts. This is the same measurement the exercise on the 10-03 lanes needed; reuse that harness, do not invent one.
3. **Concurrency.** Does the daemon bound simultaneous runs? I found no limit in `pii-server` or `pii-graph`; confirm before any watch can enqueue unbounded work.
4. **Observe mode.** Prove that with mode = observe a full run produces records and no side effect (no commit, no post), with a control that act mode does.

## 5. Recommendation

Do the four measurements first, as one small row. If 1, 3 and 4 hold, `repo-watch` plus `repo-eval` is two graphs and one row of Rust at most. If 2 fails, fall back to `route` rules (as `discord-triage` does) before touching a model. Order after rows 401 and 402; it does not block this week's plan.

## 6. Sources

- https://github.com/deepseek-ai/dsh-superagent-prototype (README, docs/002, docs/003, docs/007)
- https://offloop.org/learn/deepseek-super-agent-design (explainer; benchmark figures)
- https://pragmaticai.substack.com/p/deepseek-superagent-from-a-github (walkthrough)
- the workspace memory: `research/dsh-superagent-deepseek`, `insights/dsh-superagent-deepseek`, `insights/pii-from-pi-durable-2026-10-13`
