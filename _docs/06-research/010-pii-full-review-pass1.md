# Pii (`pi-improved`) — full-codebase adversarial review, pass 1

**Date**: 2026-09-08 · **Reviewer**: claude-opus-5 · **Target**: `/projects/pi-improved` @ `24f6427`
**Scope**: 16 crates, Phase 4 (4.1a → 4.7 partial). Every claim below cites `path:line`
and was checked against the code in this tree — several were checked against a **running
process** (marked PROVEN) rather than inferred.

**Verdict: NEEDS WORK.** The substrate is still the good part. But Phase 4.1b/4.1c
introduced three defects that, together, mean **the agent's tool loop does not function
against a real model provider**. Everything P4.7 wants to build — patch, validate,
escalate, promote — is a tool loop.

---

## A. Regressions from Phase 4

### A1 — CRITICAL (P4.1c): the agent can never execute a tool against a real provider

`AgentNode::run` now calls the streaming path:

- `crates/pii-server/src/agent.rs:193` — `self.provider.chat_streaming(ctx, &req, &mut on_delta)`
- `crates/pii-model/src/ollama.rs:228-233` — `OllamaProvider::chat_streaming` returns
  `ChatResponse { text, tool_calls: Vec::new(), ... }`. **`tool_calls` is hardcoded empty.**

Consequence: `resp.tool_calls.is_empty()` at `agent.rs:229` is *always* true, so every
model reply is treated as a final answer and the node returns on `ok` at turn 1.
`run_tool_calls` (`agent.rs:339`) is unreachable in production. `max_turns`, the tool
schemas sent in `ChatRequest.tools`, `files_changed`, the heal-fix agent's edit loop —
all dead.

Why no test catches it: the trait's **default** `chat_streaming`
(`crates/pii-model/src/lib.rs:132-141`) delegates to `chat`, which *does* return tool
calls. Every test provider (`RecordingProvider` `agent.rs:462`, `DeltaProvider`
`agent.rs:626`, `ScriptedProvider`) either uses that default or returns no tool calls.
Only the real `OllamaProvider` overrides it, and only the real one is broken.

Ollama does deliver tool calls in the stream — `OllamaMessage.tool_calls` is already
declared and `#[serde(default)]` (`ollama.rs:45-46`). The streaming accumulator simply
discards them.

**Fix**: accumulate `message.tool_calls` across NDJSON lines in `chat_streaming` the same
way `text` is accumulated (`ollama.rs:219-227`), and return them.

### A2 — CRITICAL (P4.1b): every tool that declares a `timeout()` runs with EMPTY capabilities

`crates/pii-server/src/agent.rs:412-417`:

```rust
let handle = std::thread::spawn(move || {
    tool.call(
        &Ctx::root(pii_core::CapSet::empty(), pii_core::Budget::unlimited()),
        &args,
    )
});
```

The caller's `ctx` is **discarded** and replaced with an empty-capability root context.
`CapSet::empty()` is `Default` (`crates/pii-core/src/cap.rs:153-155`) — no kinds, no
path scopes. Therefore:

- `bash` (`crates/pii-tools/src/bash.rs:85` → `ctx.proc()?`) → `PiiError::Denied`, always.
- `grep` / `glob` (`crates/pii-tools/src/search.rs:164` → `ctx.fs()?`) → denied, always.

Those are exactly the three tools that declare a timeout (`bash.rs:47-51` 120 s,
`search.rs:139-142` and `search.rs:211-214` 30 s each). The untimed tools (`fs_read`,
`fs_write`, `edit`, `multi_edit`, `lib.rs:41-43` → `None`) take the inline branch at
`agent.rs:407-409` and keep the real ctx, so they still work — which is why the Slice-1
heal e2e (fix agent with fs/edit tools and **no bash**) passed and hid this.

`Ctx` is documented `Send + Sync` and cloneable (`crates/pii-core/src/ctx.rs:36`), so the
fix is one line: clone the caller's ctx into the worker thread. Do **not** substitute a
root capset — that would turn a confinement bug into an escalation (the heal run's
`scope_root` fs_write confinement lives in that ctx).

Why no test catches it: `slow_tool_times_out` (`agent.rs:566-615`) uses a fake `SlowTool`
whose `call` ignores `ctx` entirely.

### A3 — HIGH (P4.1c + P4.4): `request_streaming` is not byte-equivalent to `request`, and streaming emits zero deltas in production

`NetService::request` decodes the body through `parse_response`
(`crates/pii-net/src/service.rs:353`), which honours `Content-Length` **and** dechunks
`Transfer-Encoding: chunked` (`crates/pii-net/src/http_codec.rs:165-194`, `read_chunked`
at `196-233`).

`NetService::request_streaming` does neither. It parses only the head
(`service.rs:395`) and then treats every remaining raw byte as body, forwarding it
verbatim to `on_chunk` and returning it as `HttpResponse.body`
(`service.rs:397-418`). No dechunking. No `Content-Length` truncation. The
`Connection: close` header (`http_codec.rs:66`) is what makes read-to-EOF terminate at
all.

**PROVEN**: the local Ollama answers a `"stream": true` chat with

```
HTTP/1.1 200 OK
Content-Type: application/x-ndjson
Connection: close
Transfer-Encoding: chunked

93
{"model":"granite4.2:3b",...,"done":false}

90
{...}
```

(raw socket probe against `127.0.0.1:11434`, this session).

So the first line of every `on_chunk` payload is a hex chunk-size line. The Ollama delta
closure (`ollama.rs:192-203`) does:

```rust
for line in String::from_utf8_lossy(bytes).lines() {
    let Ok(l) = serde_json::from_str::<OllamaStreamLine>(line) else {
        return; // partial line: the next read completes it in this model
    };
```

`"93"` is not valid JSON → `return` → the closure exits before reaching the JSON payload
on the next line. **`on_delta` never fires. `Event::AssistantDelta` is never emitted
against a real Ollama.** P4.1c's headline feature is a silent no-op in production — the
exact pathology the Phase-4 plan was written to eliminate (`007-...-plan.md:16`).

The final `ChatResponse.text` is still correct: the accumulator at `ollama.rs:219-223`
`filter_map`s unparseable lines away, and the JSON objects happen to survive as intact
lines through the chunk framing. So the user gets the right answer with no live text.

Second, independent defect in the same closure: the `return` comment is wrong. There is
**no cross-chunk line buffer**. A TCP read that lands mid-JSON-line drops that line
permanently — the tail arrives at the head of the next chunk and doesn't parse either.
The R2 acceptance test's `ChunkedTransport` (`ollama.rs:333-377`) hands three *complete,
unframed* NDJSON lines, so it cannot detect either problem.

**Fix**: dechunk inside `request_streaming` (reuse `read_chunked`'s state machine
incrementally) so streaming and buffered bodies are byte-identical, and keep a residual
`String` buffer in the Ollama delta closure across chunks. Then make the acceptance test
feed *chunk-framed* bytes split at a mid-line boundary.

### A4 — HIGH (P4.4): connection-lease leak on TLS setup failure

`crates/pii-net/src/service.rs:276-286` increments the lease counter, then:

```rust
match dial(...) {
    Ok(stream) => {
        let stream = if scheme == Scheme::Https {
            Stream::Tls(Box::new(dial_tls(stream, &target.host)?))   // <- service.rs:292
        } ...
```

The `?` returns from `connect` **before** `Conn` is constructed, so `Drop for Conn`
(`service.rs:133-140`) cannot decrement, and the compensating decrement in the dial-error
branch (`service.rs:305-311`) is not on this path. Every `dial_tls` failure — a bad SNI
name (`dial.rs:68-69`) or a rustls client-config error (`dial.rs:70-71`) — permanently
burns one lease against `grant.max_conns` for that `(principal, key)`. After `max_conns`
such failures the principal can never reach that host again for the daemon's lifetime.

**Fix**: hoist the lease decrement into a guard, or build the `Conn` first and wrap the
stream after.

### A5 — TLS cert verification: correct, but untested on the real path

`dial_tls` (`crates/pii-net/src/dial.rs:54-73`) uses `ClientConfig::builder()
.with_root_certificates(webpki_roots)` `.with_no_client_auth()` — the **default** rustls
verifier, no `dangerous()` escape hatch, SNI from the parsed host. That is right.

Two caveats:
- The handshake is lazy (`StreamOwned`), so a certificate failure surfaces later as
  `PiiError::Unavailable("net: read: ...")` from `Conn::read` (`service.rs:184-185`) —
  indistinguishable from a transient network error, and therefore likely to be treated as
  retryable by callers. A real cert rejection should not be retried.
- No test performs a real handshake; the shipped tests are `parse_url_https`
  (`http_codec.rs:288`), `parse_url_https_default_port` (`:298`) and a dispatch test.
  Byte budgets on TLS charge *plaintext* bytes (post-decrypt) while plain HTTP charges
  wire bytes, and the handshake bytes are charged to nobody. Minor, but the accounting is
  not scheme-neutral as R3 claims.

### A6 — MEDIUM (P4.1b): compaction never fires in the shape that matters

`compact` (`crates/pii-server/src/compaction.rs:57-88`) defines a "turn" as a
`Role::User` message (`:69-76`). Inside one agent run there is exactly **one** user
message; everything after it is Assistant/Tool pairs. So `n_turns` is 1, the guard
`n_turns <= keep_recent_turns` (default 6, `compaction.rs:26-29`) hits at
`compaction.rs:78-83`, and `compact` returns the input **unchanged**.

Result: a long tool loop (dozens of tool results, each up to 64 KiB) sails past the
threshold; `needs_compaction` keeps returning `true` at `agent.rs:159`, `compact` keeps
returning the same list, nothing ever shrinks, and the context overruns `num_ctx` anyway.
The one shape a coding agent actually produces is the one compaction cannot handle.

Secondary: if it *did* split there, an Assistant message carrying `tool_calls` can be
collapsed into the summary while its matching `Role::Tool` result survives in `recent` —
an orphaned tool result. Harmless against Ollama only because of A7.

### A7 — MEDIUM: tool-call structure is dropped on the wire

`message_value` (`crates/pii-model/src/ollama.rs:237-245`) serialises **only** `role` and
`content`. `ChatMessage.tool_calls` and `tool_call_id` (`lib.rs:47-52`) are never sent.
So after a tool round-trip the model sees a `role: "tool"` message with no linkage and no
record that it ever requested the call (a tool-call turn usually has empty `content`).
Multi-step tool use degrades to the model re-requesting the same tool. This becomes
load-bearing the moment A1 is fixed.

### A8 — env scrub: works, and is a known blocker for the GitHub slice

`CHILD_ENV_ALLOWLIST` (`crates/pii-core/src/childproc.rs:197`) keeps
`PATH/HOME/LANG/LC_ALL/TERM/PWD/TMPDIR` plus every `PII_*`, applied at both `Command`
sites (`childproc.rs:42` and `childproc.rs:258`). `bash` works: `/bin/sh`, `cargo` and
`git` all resolve from `PATH` + `HOME`. Secrets are genuinely dropped
(`run_child_scrubs_environment`, `childproc.rs:347`).

Two notes:
- **Forward blocker**: `SSH_AUTH_SOCK`, `GIT_*`, `GH_TOKEN`, `SSL_CERT_FILE`, `CARGO_*`,
  `RUSTUP_*` are all dropped. That is correct today and is exactly right for a model's
  shell. It also means the P4.7 GitHub slice cannot `git push` or run `gh` from the bash
  tool. Decide the credential path now: inject per-call via the existing `extra` env pairs
  on `spawn_child` (`childproc.rs:176`), scoped to a dedicated node kind — **do not widen
  the allowlist**.
- Cosmetic dead code: `ChildProc::spawn` applies `extra` twice, via
  `scrubbed_env(&live_env(), env)` (`childproc.rs:42`) and again in the loop at
  `childproc.rs:49-51`.

### A9 — session durability (P4.2): sound, with two real growth/atomicity issues

Verified correct:
- ids come from a durable monotonic counter — `Engine::create_session`
  (`crates/pii-server/src/lib.rs:247`) → `Store::next_session_id`
  (`crates/pii-store/src/session.rs:113`). No `len + 1` collision.
- boot re-seed (`session_durability.rs:146-158`) and `session.list`
  (`session_durability.rs:47-57`) are consistent: the list reads the RAM map, which the
  boot seed fills.
- `reconcile_boot` (`session_durability.rs:165-184`) genuinely clears zombies: it sets
  `header.status = Failed` then calls `append_event`, and `append_events` repoints
  `RUN_BY_STATUS` from `header.status` in the same transaction
  (`crates/pii-store/src/lib.rs:266-282`). No stale index.
- no redb second-opener risk: one `Store`, one process.

Two defects:
- **(a) O(n²) writes per session.** `persist_session` (`session_durability.rs:38-44`)
  serialises the *entire* `Session` — including the full transcript — and it is called on
  every assistant turn (`session_durability.rs:129`). A 200-turn boss session rewrites the
  whole transcript 200 times. Combined with the append-only journal this is the second
  unbounded write amplifier.
- **(b) lost token counts.** `record_assistant` does a non-atomic read-modify-write of the
  session token totals (`session_durability.rs:95-105`). Two runs finishing concurrently
  in one session lose one update. `session.state` then under-reports, which is the exact
  class of lie P4.2 was built to remove.

### A10 — `spawn_resume` restores `scope_root` but does not re-validate it

`runs.rs:138` recovers the scope from the journal; `spawn_run` validates
(`runs.rs:42-44`) but `spawn_resume` does not call `validate_scope_root`
(`runs.rs:237-259`). Defense in depth against a workspace-config change between start and
resume; a two-line fix.

---

## B. Is the substrate sound for a long-running autonomous boss?

The engine, journal, capability intersection and sandbox are sound and I would build on
them. The layer the boss actually runs on is not ready. Highest-risk failure modes over a
multi-hour/multi-day run, ranked:

**B1 — The boss cannot act at all (A1 + A2).** It will produce a plausible plan, request
`bash`/`grep`, and the tool call will either never reach the loop (A1) or return
`capability denied` (A2). Because the run still terminates `Success` on the `ok` port, the
journal will show a *clean, successful* run that changed nothing. This is the worst
possible failure shape for an autonomous system: it looks healthy while doing nothing.
Nothing else on this list matters until it is fixed.

**B2 — No brake on spend or effort.** `BudgetKind::MicroUsd` is *read* once
(`crates/pii-graph/src/run.rs:265`) and **never charged** anywhere in the workspace.
`BudgetKind::ToolCalls` is defined (`crates/pii-core/src/budget.rs:13`) and never charged
either. `max_model_calls` still does not exist in any crate (only in
`specs/06-phase3-mvp-hardening.md:110`). The only live dimension is `Tokens`
(`agent.rs:214-228`), and on the streaming path its counts come from the last NDJSON line
(`ollama.rs:224-232`) — a parse miss silently charges zero. A looping boss on a paid model
has no enforceable ceiling.

**B3 — Unbounded durable growth, no retention.** Three append-only stores with no prune
verb: `RUN_JOURNAL` (by design, documented in `HANDOFF.md`'s rss-gate section), the
per-turn full-transcript session rewrite (A9a), and `HealAttempt.attempts`
(`crates/pii-store/src/heal.rs:37`) which only grows per signature. A daily boss is the
first workload that runs for a month; `pii admin prune` was in the P4.2 plan
(`007-...-plan.md:80`) and was not built.

**B4 — The human-approval contract is not enforceable inside a loop (see E2).** The boss
loop *is* a loop. A gate node re-entered on iteration 2 of the same run reads its own
cached decision and auto-allows. The user's #1 decision ("human approves every promotion")
does not hold in the shape the boss uses.

**B5 — One panic takes down everything, and nothing restarts it.** `panic = "abort"` in
release plus a single daemon means a panicking node kills every concurrent session,
including the boss's own state. Post-P4.2 the crashed run at least surfaces as `Failed`
(`session_durability.rs:165`), but recovery depends on the container `entrypoint.sh` hook
landed in P4.5 — there is no supervision loop that notices "the boss stopped" and says so.

**B6 — Nothing stops the boss from editing its own reward signal.** No guard anywhere in
`crates/` or `xtask/` refuses a patch touching `xtask/`, `quality-gate.sh`, `graphs/`,
`specs/`, `.github/`, or a `#[test]` assertion. The gates *are* the boss's fitness
function; today it has write access to them whenever its `scope_root` is the workspace.

---

## C. `HealStore` schema for the boss's fixing-analysis ledger

**Not enough — it needs the attempt → patch → gate → decision evolution.**

Current schema (`crates/pii-store/src/heal.rs:13-38`): `HealAttempt { signature_hash,
gate, first_seen_ms, last_seen_ms, attempts: Vec<HealAttemptRun { run_id, outcome:
String, ts_ms, detail }> }`. It records **that a gate failed and was seen again**. That is
the right shape for Slice 0 (detect-only) and it is genuinely well-built: blake3 signature
key so repeat failures update one row, no held transaction, bounded cache.

It cannot answer any question the boss's ledger must answer: *what did we change? did it
work? who said yes? what did it cost?*

Recommended evolution (additive; the key stays `signature_hash`):

- `HealAttemptRun` gains:
  - `patch_hash: Option<String>` — the diff stored **content-addressed** in the existing
    blob table (`Store::put_graph`/`get_graph` is already a CAS, `pii-store/src/lib.rs`),
    not inline. Inline diffs would make the row unbounded.
  - `worktree: Option<String>` + `branch: Option<String>` + `base_commit: Option<String>`
    — the scope the fix was made in, so the artifact is reproducible.
  - `gate_before: String`, `gate_after: Option<String>` — the outcome of the *same* gate
    re-run after the patch. This is the single most important missing field: without it
    the ledger cannot distinguish "we tried" from "we fixed it".
  - `decision: Option<Decision { allow: bool, approval_id: String, by: String, ts_ms: u64 }>`
    — bound to the gate node's `approval_id` (`crates/pii-server/src/nodes/gate.rs:101`)
    so the ledger and the approval store agree on one identity.
  - `model: String`, `tokens_in: u64`, `tokens_out: u64`, `micro_usd: u64` — the cost of
    the attempt. This is also what makes B2's budget dimensions chargeable with real data.
- `HealAttempt` gains `state: HealState` — a real enum
  (`Detected | Patched | Validated | Rejected | Approved | Promoted | Reverted`), replacing
  today's stringly-typed `outcome: String` (`heal.rs:18`).
- `attempts` becomes a **bounded ring** (last N, default 20) with a separate
  `attempt_count: u64`. Otherwise a signature the boss retries daily grows one row
  forever (B3).

Do this **before** the promotion slice, not after: the promotion gate's audit trail is the
only reason a human can meaningfully approve anything.

---

## D. The `pii-mcp` client hang — actual cause and minimal fix

**The `HANDOFF.md` caveat is misdiagnosed. The client is not broken; the server never
exits.**

I ran the client against a `tee`-wrapped server and captured both directions of the wire.
The full protocol round-trip completes and the client exits **0**:

```
client -> server:
  {"jsonrpc":"2.0","id":0,"method":"initialize",...}
  {"jsonrpc":"2.0","method":"notifications/initialized"}
  {"jsonrpc":"2.0","id":1,"method":"tools/list",...}
  {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{...,"name":"bash"}}
server -> client:
  initialize result, 7 tools, and a tools/call result
client stdout:
  tools: ["bash","edit","fs_read","fs_write","glob","grep","multi_edit"]
  call bash: Complete(CallToolResult { ... "error: invalid: command must be a string" ... })
  CLIENT_RC=0
```

(The `bash` error is just the smoke test calling the first tool with no arguments —
`run_client` sends `CallToolRequestParams::new(name)` with no args,
`crates/pii-mcp/src/main.rs:189`.)

**Root cause**: the server's `main` parks forever —
`std::future::pending::<()>().await` at `crates/pii-mcp/src/main.rs:155`. The
`RunningService` correctly keeps the server alive, but *nothing ever resolves*, so
`pii-mcp` does not exit on stdin EOF or on transport close. Meanwhile `run_client` builds
the child with `tokio::process::Command` and only `TokioChildProcess` pipes stdin/stdout
(`main.rs:169-171`) — **stderr is inherited**. So after the client returns, the spawned
server survives as an orphan holding the *invoking shell's* fd, and the pipeline never
sees EOF. The shell hangs; the client did not.

PROVEN: four orphaned `pii-mcp` processes with `ppid == 1` are alive in this container
right now (started 04:23, 11:30, 11:34 during the P4.7 session, plus 11:40 from my own
probe). The `tee`-wrapped run left no orphan only because `tee` exits at EOF and SIGPIPEs
the server.

**Minimal fix** (both halves, ~4 lines):

1. Server: replace `std::future::pending::<()>().await` (`main.rs:155`) with
   `let _ = running.waiting().await;` — `RunningService::waiting` exists in this exact
   dependency (`rmcp-3.2.0/src/service.rs:1093`) and resolves with a `QuitReason` when the
   transport closes. Bounded server lifetime, clean exit code, no orphans.
2. Client: set `command.stderr(std::process::Stdio::null())` before
   `TokioChildProcess::new` (`main.rs:169-171`), and `let _ = running.cancel().await;`
   (`rmcp-3.2.0/src/service.rs:1162`) before `run_client` returns, so the child is
   explicitly torn down rather than orphaned.

With (1) in place the self-test becomes a genuine exit-code assertion and can be an
exit criterion, not a manual smoke test. Also worth fixing while there: pass real
arguments to the smoke call (e.g. `fs_read` on a temp file) so the test proves a tool
*ran*, not that an error round-tripped.

---

## E. Prior opus review items — current status

| # | Item | Status |
|---|---|---|
| 1 | resume restores `scope_root` | **FIXED** |
| 2 | approval-gate cycle salt | **PARTIAL — the boss's shape is still broken** |
| 3 | `heal-watch.toml` phantom gate | **FIXED** |
| 4 | `run_ci` missing reachability/doc-drift | **STILL OPEN** |
| 5 | anti-reward-hacking refusal | **NOT PLANNED — no code, no spec** |
| 6 | dev/prod runtime-dir vs worktree | **Recorded as a constraint, no code** |

**E1 — resume scope-root: FIXED.** `spawn_resume` recovers the scope from the journal
(`crates/pii-server/src/runs.rs:138`) via `stored_run_scope_root`
(`runs.rs:177-195`), which reads `RunEvent::Started { input_hash }` and resolves it
through the CAS. The real path does store the input content-addressed
(`crates/pii-graph/src/run.rs:87` → `Started { input_hash }` at `run.rs:99`), so the test
fixture matches production. Two tests (`runs.rs:352`, `runs.rs:395`). Residual gap: no
re-validation on resume (A10).

**E2 — approval-gate cycle: PARTIAL, and the remaining hole is exactly the boss's shape.**
`approval_id` now folds a `cycle` salt (`crates/pii-server/src/nodes/gate.rs:101-116`),
cycle 0 preserves the historical `blake3(run:node)` id so §8.8 replay is unchanged
(test `gate.rs:153-170`), and a different cycle yields a different id
(test `gate.rs:141-148`). That is all correct.

But `cycle` is a **static graph-config field read at build time**
(`gate.rs:56-59`) and nothing increments it at runtime. Inside a single run — which is
what a boss *loop* graph is — a gate node re-entered on iteration 2 has identical
`(run_id, node_id, cycle)`, finds its own cached decision (`gate.rs:79-87`), and
**auto-allows without a human**. The original finding stands for the exact topology P4.7
needs. Getting a fresh cycle today requires spawning a new run from a *different* graph
spec each cycle (the spec is content-addressed, so a changed `cycle` is a new graph id) —
workable, but it is not what "the boss loop" means and nothing implements it.

**Fix**: derive the salt at runtime, not from config — e.g. fold the loop-iteration
counter or an explicit `cycle` value read from `input.payload` into `approval_id`, so a
re-entered gate is always a new decision. Keep the config field as the default of 0.

**E3 — heal-watch: FIXED.** `graphs/starter/heal-watch.toml:27` is now
`gate = "reachability-check"`, a real xtask subcommand. The old `quality-gate` value
would have recorded a phantom permanent failure every 30 minutes.

**E4 — `run_ci` reachability/doc-drift: STILL OPEN.** `xtask/src/main.rs:63-81` runs
deps-check, net-check, lint-files, proto-gen, bench, fmt, clippy and test. Neither
`reachability-check` nor `doc-drift-check` is in the CI gate set — so the two gates the
Phase-4 plan called "the strongest self-improvement machinery in this repo"
(`007-...-plan.md:122`) are still opt-in and not enforced by the gate the boss will be
told to satisfy. This is a one-line fix and it is the cheapest item on this list.

**E5 — anti-reward-hacking: NOT PLANNED.** No refusal logic exists anywhere in `crates/`
or `xtask/`, and no spec mentions it. Given B6 and E4 together, a boss whose patch is
scored by a gate it can edit is a genuine hazard, not a theoretical one. This should be a
mechanical check on `git diff --name-only` inside the heal/promotion graph — escalate,
never propose — and it is cheap because the heal loop already has a worktree and a gate
node.

**E6 — dev/prod runtime-dir vs worktree: constraint recorded, correct, no code.**
`HANDOFF.md` caveat 7 matches the code: `validate_scope_root`
(`crates/pii-server/src/runs.rs:237-259`) canonicalises both sides and rejects anything
not under the workspace, so a `/root/.pii-dev/` worktree can never be a heal
`scope_root`. Keeping `/root/.pii-dev/` as a runtime/store dir only, with worktrees under
the workspace, is the right call and remains unimplemented.

**Also still open from the Phase-4 plan itself**: `MicroUsd` never charged (B2),
`max_model_calls` absent (B2), retention/prune absent (B3), `pii-evolve` crate absent
(`crates/` has no `pii-evolve`).

---

## F. Highest-leverage next P4.7 step, and its single biggest risk

**Step: not `pii-evolve`. A "P4.7.0 — the agent loop actually works, proven live" slice.**

Contents, in order:
1. A1 — accumulate `message.tool_calls` in `OllamaProvider::chat_streaming`
   (`ollama.rs:228`).
2. A2 — pass the caller's `Ctx` (cloned) into the bounded tool thread
   (`agent.rs:412-417`).
3. A3 — dechunk inside `NetService::request_streaming` (`service.rs:395-418`) and buffer
   partial NDJSON lines across chunks in the Ollama delta closure
   (`ollama.rs:192-203`).
4. E4 — add `reachability-check` + `doc-drift-check` to `run_ci` (`xtask/src/main.rs:63`).
5. D — bounded MCP server lifetime (`main.rs:155`) so the self-test is an exit code.

And **one non-negotiable exit criterion**: a live run against the local Ollama in which
the agent calls `grep` and then `bash`, and `pii graph trace <run_id>` shows `ToolCall` +
`ToolResult` events and at least two `AssistantDelta` events. Not a fake transport. Not a
scripted provider.

Rationale: every P4.7 capability — localize, patch, validate, escalate, promote, research,
open a PR — is a tool loop over `bash`/`grep`/`edit`. Building the `pii-evolve` crate on
top of a loop that returns at turn 1 with `Terminal::Success` would produce a boss whose
journal is full of clean successful runs that changed nothing, and the failure would be
attributed to the model, the prompt, or the graph for a long time before anyone suspected
the substrate.

**Single most important risk of that step: it is only provable with a live model, and the
existing test apparatus is structurally incapable of catching these bugs.** All three
defects are invisible to the 52 test binaries because every fake bypasses the broken path:
the default `chat_streaming` delegates to `chat` (`pii-model/src/lib.rs:132-141`),
`ChunkedTransport` hands unframed NDJSON (`ollama.rs:333-377`), and `SlowTool` ignores its
`ctx` (`agent.rs:572-594`). If this fix is validated the way it was written — with fakes —
it will **vacuous-pass**, exactly as spec 16 part 2 and spec 20 already did per
`HANDOFF.md` caveat 8. The gate must be a live run, and the new acceptance tests must
model the wire: chunk-framed bytes split mid-line, and a tool that actually enforces
`ctx.caps()`.

---

## One-line verdict

**NEEDS WORK** — the single most important thing to fix next is the agent tool loop
(A1 `chat_streaming` drops `tool_calls`, A2 `call_tool_bounded` substitutes an empty
`CapSet`), gated on a **live-model** exit criterion; do that before any `pii-evolve` code,
because today every autonomous run would report success while doing nothing.
