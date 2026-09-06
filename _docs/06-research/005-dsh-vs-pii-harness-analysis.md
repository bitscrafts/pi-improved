# DeepSeek Harness (dsh) vs Pii — full comparative analysis

**Date**: 2026-09-06 · **Author**: orchestrator (research + analysis) ·
**Status**: ANALYSIS COMPLETE — spec 11 (journal hardening) extracted and
user-approved for pidag implementation; items 3-5 deferred.

## 1. What this is

A grounded comparison of **DeepSeek Harness** (`dsh`, MIT, ~212k GitHub
stars in its first days, developer preview) against **Pii**
(pi-improved, this repo), to decide what dsh has that pi-improved should
steal — and what dsh's hype glosses over.

Sources: github.com/deepseek-ai/deepseek-harness (README,
docs/architecture.md, docs/subsystems/{session,tools,persistence}.md,
SAFETY.md), deepseek.com/harness/en, venturebeat/aitoolsreview coverage,
plus pi-improved source reading (this repo) and prior agent-memory
findings (workspace/architecture/append-only-event-log-pattern,
workspace/research/deepseek-harness-qwen38,
workspace/insights/harness-matters-more-than-model).

## 2. What DeepSeek Harness actually is

- **MIT-licensed agent harness, TypeScript/Node.js monorepo** —
  `npx @deepseek-ai/dsh web` runs a Web UI at 127.0.0.1:3080; also a
  `headless` one-shot profile. Developer preview: "THERE WILL BE
  COMPATIBILITY-BREAKING CHANGES"; ships a SAFETY.md; prior research
  (2026-08-29, agent-memory) records plugin bugs bricking installs,
  mistimed compaction, frequent Windows errors.
- **Everything-is-a-plugin** — built on **Cordis**
  (github.com/cordiverse/cordis; paper arxiv.org/abs/2608.25512,
  "A Programming Paradigm for Spatiotemporal Composability"). Model
  adapters, tools, skills, sessions, sandboxes, storage, loops,
  scheduling AND the UI are all plugins. "There is no privileged core
  to patch": registrations are reversible effects that unwind when a
  plugin unloads (temporal composability = revertible effects; spatial
  = reactive co-effects).
- **Profiles & bundles** — a running dsh is a plugin tree composed from
  ordered config layers: each profile stacks bundles (dsh-base first:
  model adapters, tools, persistence, sandbox/approval, credentials,
  telemetry; then web-app or headless), then user `cordis.patch.yml`
  layers, then `--patch` overrides. `dsh --dump-config` prints the
  entire bootable tree; any row is replaceable via patch. A **bundle**
  is a distributable set of config rows + code.
- **Session = append-only event log** (`core/session`): a Session is an
  append-only log of typed SessionEvents — the single source of truth
  for the whole interaction history. The LLM message history is
  DERIVED by pure projection, never stored separately; replay is
  re-derivation. No editing: "stability is emergent" — cache stability
  is a corollary of the log-you-cannot-edit. Worth ~120x in token cost
  on DeepSeek Pro ($0.0033/M cached vs $0.435/M uncached, from the
  2026-08-13 research).
- **Persistence = a capability seam** (`session-persistence`): one
  abstract service (`create/open/stat/list`) with a shipped JSONL+zstd
  backend and pluggable alternatives. Log access flows through a
  **SessionHandle** (read/append/flush/close) that is the single door;
  single-writer ownership — a second `open(id, 'write')` rejects with
  SessionAlreadyOwnedError. Appends are best-effort (bounded
  write-behind window, ~50-100ms batching); **flush() is the one
  durability barrier** the loop uses as its ordering/error checkpoint
  before the next turn. A rejected background write retains its events
  in order and pauses the automatic path.
- **Crash recovery = reader-side repair**: a log ending mid-turn
  (`turn/start` with no `turn/end`) is NOT truncated or repaired by the
  writer. Resume reads the stored log, computes `interruptedTurnClosers`
  (missing tool errors, any open step/end, a synthetic
  `turn/end { reason: { kind: 'interrupted' } }`) and appends them
  through the write handle as an ordinary batch. `interrupted` is the
  one TurnEndReason no loop emits itself. Repair writes only under
  write ownership — read-only observers balance the same closers in
  memory, writing nothing.
- **Tools** (`core/tools`): typed ToolDefinition registry; the
  model-facing ToolSchema is built by an explicit allowlist so host-only
  fields (execute, finalizeContent, timeoutMs, isConcurrencySafe, UI
  presenters) never leak into a model request.
- **Real-world signal** (agent-memory 2026-08-29, Kai video analysis):
  "the harness matters as much as the model" — same Qwen3.8-27B weights
  failed in a bare harness (black window, plan but no output) and worked
  FIRST attempt inside dsh; Qwen's own headline agentic benchmark gains
  (13.3->42.2 DeepSWE) were run inside the CLAUDE CODE harness, not the
  shipped one. But: dev-preview quality; small models on EXISTING
  codebases loop for dozens of hours re-planning without committing
  (greenfield OK, medium existing codebase NOT); 27B local model on 2x
  used 3090s built an FPS game unattended over 5h (687 steps, ~88M
  tokens in / 822K out) while the dev slept.

## 3. Direct comparison to Pii

### 3.1 Convergent architecture — VALIDATES our design (not gaps)

| Concern | dsh | pi-improved | Verdict |
|---|---|---|---|
| Session/history model | append-only SessionEvent log; history derived by projection | Run journal = append-only RunEvent log (seq); resume = full replay (crates/pii-graph/src/journal.rs, run.rs) | SAME pattern, independently converged (agent-memory scored this convergence 0.95 on 2026-08-15) |
| Durable persistence seam | abstract SessionPersistence + JSONL backend + handle/single-writer | pii-store::Store (redb); one append transaction; content-addressed inputs/outputs | Both real; pi uses redb (single-file, ACID) vs dsh JSONL+zstd |
| Tool registry vs model schema | allowlist projection ToolSchema vs ToolDefinition | pii-tools Tool trait: `spec()` (model-facing) vs `call()` (execution) — separated by construction | pi is arguably STRONGER: the Rust type split makes leakage unrepresentable, no runtime allowlist to get wrong |
| Capability attenuation | sandbox/approval plugins | Ctx capability gates; CapKind intersection at 3 levels (run caps ∩ graph caps ∩ node caps) | pi matches or exceeds (proven live this week: the two-layer NET-cap fix) |
| Append-only log = cache stability | 120x token-cost win, prefix cache | Same journal-replay prefix property applies to any future cached provider | Shared insight, already documented |

### 3.2 Real gaps dsh exposes in pi-improved

**(a) Plugin composability / no privileged core.** dsh's headline:
everything swappable, registrations reversible, user patch layers
override any row. pi-improved: `NodeRegistry::builtin()` is fixed at
compile time; new node kinds require a Rust change in
crates/pii-server/src/registry.rs. Node "plugins" exist conceptually
(NodeRegistry::register) but nothing is dynamically loadable or
overridable at runtime; no graph/config layering beyond hand-editing
TOML. Upgrade path (aligns with pi-improved's own WASM-plugin goal):
runtime-loadable node factories (WASM or dlopen) behind the EXISTING
NodeFactory trait + per-profile graph layering (base graph -> user patch
-> CLI override). NodeFactory is already the seam; the missing piece is
dynamic discovery + override ordering. **Spec-first; the big one.**

**(b) Interrupted-turn closer on resume.** dsh treats a crash mid-turn
as first-class: resume appends synthetic closers so the model/operator
sees WHY the previous turn ended. pi-improved's resume
(crates/pii-graph/src/run.rs:124) replays the journal, appends a bare
`Resumed`, and continues. Dispatch journals `NodeStarted` BEFORE running
(dispatch.rs:193, "a node interrupted mid-flight ... resume re-runs
it"), and replay ignores a dangling `NodeStarted` (run_helpers.rs:60,
"that node re-runs") — but the journal never RECORDS the interruption:
a resumed trace shows `NodeStarted` ... then a NEW `NodeStarted` for the
same node, with no terminal event between, inexplicably. **Upgrade:
synthetic `RunEvent::NodeInterrupted { node }` appended on resume for
each dangling node (seq order, before Resumed); audit marker only —
replay treats it as a no-op, node still re-runs.**

**(c) Bounded write-behind + flush barrier.** dsh batches appends in a
bounded window with an explicit flush() durability barrier. pi-improved
appends EVERY journal event in its own redb write transaction
(pii-store append_event: open, insert, rewrite header, repoint status
bucket, commit — per call). Deliberate ("no transaction is ever held
across a node dispatch" — journal.rs) and safe, but it is the dominant
growth driver of the run journal under bursty node chatter, and a big
share of the RSS slope `xtask rss-gate` measures — the same slope that
required the `SLOPE_KB` threshold raise from 2048 to 16384 (commit
`587c5d1`, flagged NEEDS-REVIEW). **Upgrade: batch N events per redb
write transaction (Store::append_events) with an explicit flush barrier
at the durability points (terminal finish, suspend, status
transitions); keep the no-transaction-across-dispatch invariant; crash
loses only unflushed events (valid contiguous prefix, header stays
Running — recovery unchanged). This is the legitimate fix that can
retire the flagged threshold change.**

**(d) Single-writer session ownership.** dsh's SessionHandle rejects a
second concurrent writer per session. pi-improved's Engine::sessions
map (parking_lot Mutex) is process-local; two daemons sharing a
store.redb could interleave appends to the same run/session undetected.
Theoretical today (single daemon), real the moment cron/queue/serve
grows. **Upgrade: writer-lease row per active run in the store (claim
on spawn, release on terminal/suspend; second claim = explicit error).**
Cheap; defer until multi-daemon concurrency actually arrives.

**(e) Per-tool timeoutMs / concurrency-safety.** dsh registers per-tool
timeoutMs, isConcurrencySafe, UI presenters as first-class ToolDefinition
fields. pi-improved's Tool trait has name/spec/call only — no per-tool
timeout (a runaway `bash` call today is bounded only by the node/run
budget), no concurrency-safety marker (concurrency control lives at the
graph scheduler level via Effects, not the tool level). **Upgrade: add
`timeout()` (default) and `concurrency_safe()` to the Tool trait; honor
timeout in dispatch. Small trait extension.**

### 3.3 Where pi-improved is ahead — do NOT copy dsh wholesale

- **Language/runtime**: dsh = Node.js monorepo (pnpm, huge dep surface,
  dev-preview churn, plugin-bricking reports, SAFETY.md). pi-improved =
  single static Rust binary, workspace lints -D warnings, 450-line
  file cap, no runtime install surface. For the lnx deployment (one
  binary at /root/.local/bin/pii) pi's choice is the better fit; dsh's
  plugin-tree economics buy little when you ship one binary.
- **Graphs**: dsh is a single-agent loop with plugins; pi-improved has
  a REAL graph engine (nodes/edges/ports/join policies/loop_budget/
  route — see specs/10-graph-visualize.md, `pii graph show`). The
  research thread "graph engineering" (agent-memory 2026-09-04:
  harness = execution env; loop = single-agent plan-execute-verify;
  graph = multi-node with private context + shared state + explicit
  routing) places graphs ABOVE harness loops — pi already implements
  the layer dsh would need Cordis plugins to approximate.
- **Model-agnostic providers**: pi has ollama:/scripted: providers live
  and deepseek: blocked only by the pre-existing pii-net TLS gap (B4,
  HANDOFF OPEN entry). dsh's plugin adapters are trivial to extend but
  the project narrative is DeepSeek-centric.

## 4. Upgrade ranking (agreed with user, 2026-09-06)

1. **(c) Bounded write-behind in the Journaler** — HIGH value:
   legitimate rss-gate fix (retire the flagged SLOPE_KB=16384
   threshold with evidence), no API change, contained. **USER-APPROVED
   — spec 11.**
2. **(b) Interrupted-turn closer on resume** — small; resume
   correctness + auditability. **USER-APPROVED — spec 11.**
3. **(e) Per-tool timeout + concurrency-safe marker** — small trait
   extension; bounds runaway bash. ("the remaining worth a try?" —
   user asked; NOT yet spec'd, deferred after 1+2.)
4. **(a) Dynamic node-kind loading (WASM)** — the big one; aligns with
   pi-improved's original WASM-plugin goal; spec-first; NOT scheduled.
5. **(d) Cross-process run/session write lease** — defer until
   multi-daemon concurrency arrives; document as known-limitation.

## 5. Spec extracted: specs/11-journal-hardening.md (committed b60a763)

Journal hardening = items 1+2 above, one spec, pidag implementation
(worker deepseek/deepseek-v4-flash, thinking off). Design anchors:

- **J1/J2 (write-behind)**: `Store::append_events(&mut header,
  &[RunEvent])` = one redb write txn for N events (single append_event
  becomes a thin wrapper — observable behavior unchanged); Journaler
  buffers appends (observer notified immediately at append time — the
  TUI/subscribe bus reads the observer, not the store — seq() advances
  in memory exactly as today) and flushes on: batch threshold (default
  16, constructor param for tests), set_status(), snapshot_now()
  (suspend), terminal finish (run()/resume() must return with a fully
  durable journal — every post-run reader depends on it), and drop.
  Crash loses ONLY unflushed events (valid contiguous prefix; header
  status Running; spawn_resume boot recovery unchanged). Do NOT flush
  on the Cancelled path (durability doesn't matter for a run that
  never finishes — that's the whole point of batching).
- **J3 (interrupted closer)**: new `RunEvent::NodeInterrupted { node }`
  (pii-abi journal enum, additive); resume() detects nodes whose last
  event is NodeStarted with no later terminal event anywhere in the
  journal, appends NodeInterrupted per node in seq order BEFORE
  Resumed; replay ignores it (no-op arm) — node re-runs exactly as
  today; clean resumes append nothing (common case unchanged);
  `pii graph trace` gains a human line for it. Anchor test already
  exists: `resume_never_reinvokes_completed_node` (crates/pii-graph/
  tests/resume.rs:67) cancels mid-wait and must keep passing (extended
  to assert the new marker).
- **J4 (payoff)**: full 300s rss-gate run once at the end, compare
  slope_kb vs the ~13.8MB baseline — evidence to propose retiring the
  SLOPE_KB=16384 flagged threshold (user's call, threshold untouched by
  the spec). Short 60s probe only in the machine-checked ECs (pidag
  re-runs ECs every validate cycle; a 6-minute gate would add 6+
  minutes per iteration).

## 6. Verdict

dsh validates pi-improved's core architecture decisions (event-sourced
journal, derived state, capability attenuation, append-only durability)
done in TypeScript-with-plugins; pi-improved is the same pattern in a
static Rust binary with a real graph engine on top. The transferable
upgrades are the engineering details dsh has refined further (write-
behind batching, reader-side interrupted-turn repair, per-tool
timeouts, single-writer leases) — not its plugin metaphysics. Agent-
memory insight (2026-08-15, importance 0.95) holds: append-only log +
pure projection is the emergent standard for agent state management —
two independent implementations (dsh, pidag/pi) now validate it.

## 7. Record of where this lives

- This file: `_docs/06-research/005-dsh-vs-pii-harness-analysis.md`
  (SSOT for the full analysis).
- Condensed findings + insight: agent-memory backend
  `pi-improved/research/dsh-vs-pii-harness-comparison` and
  `pi-improved/insights/dsh-validates-journal-not-metaphysics`.
- Earlier research it builds on: agent-memory
  `workspace/architecture/append-only-event-log-pattern`,
  `workspace/research/deepseek-harness-qwen38`,
  `workspace/insights/harness-matters-more-than-model`.
