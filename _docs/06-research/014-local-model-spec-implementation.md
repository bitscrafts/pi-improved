# Local models (granite/laguna) as the SDD workhorse — feasibility

**Date**: 2026-09-11 · **Author**: sonnet (research) · **Status**: RESEARCH — no decision, no code touched

## 0. Scope and method

This asks a narrower, more operational question than prior docs: not "can granite
run inside Pii" (settled — [015](015-llm-in-process-llama-cpp.md),
[023](023-llamacpp-granite-on-pi-process.md), specs 35/36) but "can granite/laguna
replace the cloud workhorse (`ollama-cloud/glm-5.3-flash:cloud`, per
`.pidag/config.toml`) that `pidag sdd` dispatches spec implementation to."

Method: this repo already ran the experiment. Spec 36 (in-process tool-call
support for the `LlamaCppProvider` itself) was attempted against granite and
laguna as full one-shot specs, failed, was split into two micro-specs
(`36-part1-llamacpp-tools.md`, `36-part2-integrate.md`), and **succeeded** —
laguna implemented part 1, committed as `a37ac8a`/`5c9cc74`. That run's logs,
briefs, and diffs are still in `_tmp/` and are the primary evidence here, not
a fresh probe. Where the repo lacks direct evidence (pidag's own source is not
checked out here — this research agent is sandboxed to `/projects/pi-improved`
and `pidag` is not on `PATH`), that gap is flagged rather than guessed at.

## 1. Context recap (read, not re-litigated)

- [004](004-granite-nano-tier-probe.md) established the non-negotiable gate:
  granite/laguna are **reasoning models** that emit `reasoning_content` and
  empty `content` unless thinking is turned off (`--reasoning off` on
  llama.cpp; **not controllable on Ollama** — it replaces the model's own
  Jinja template).
- [023](023-llamacpp-granite-on-pi-process.md) and
  `_docs/02-guides/pi-local-llamacpp.md` document the working setup: one
  llama.cpp **router** (port 8082, `--models-preset llama-models.ini
  --models-max 2 --jinja`) serving both `granite-4.2-3b` and `laguna-xs-2.1`,
  reached through pi's built-in `llamacpp` provider (base URL overridden in
  `/root/.pi/agent/models.json`).
- AGENTS.md: the SDD loop is `pidag lint` → `pidag sdd specs/NN-x.md --run
  --concurrency N` → quality-gate → validate, up to `max_iterations = 4`
  (`.pidag/config.toml`). The workhorse today is
  `ollama-cloud/glm-5.3-flash:cloud`, also gated on `reasoning_effort="none"`
  for the identical empty-content trap.
- Specs 35/36 themselves are the test subject *and* were written to be
  implemented by a workhorse — they follow the mandatory SDD format (Verified
  Premises, Requirements, TDD Contract, machine-checked Exit Criteria,
  Guardrails, Files to Modify) documented in AGENTS.md and visible in
  `specs/31-protected-paths.md` (121 lines, 5 files, file:line-anchored
  premises — a representative "real" spec).

## 2. Feasibility: what actually happened when granite/laguna tried

Four real attempts exist in `_tmp/`, in chronological order:

### 2a. Full spec 36, granite, one-shot — **failed (confused, not crashed)**

`_tmp/run-36-granite-tool-calls.log` (granite asked to implement spec 36 —
grammar-constrained tool calls, ~95-line spec, against the existing 352-line
`llamacpp.rs`) — granite wrote a file, then talked itself into a corner:

```
The file was written but there are several issues:
1. Missing `encoding_rs` import
2. `schema_builder::build_schema_from_tools` expects `ToolSpec` but the spec
   doesn't define this type (it's in common feature)
3. The parsing logic needs refinement
4. The placeholder tests should be proper TDD tests
```

`encoding_rs` is not a real dependency of this spec — granite hallucinated an
import requirement. The retry (`run-36-granite-tool-calls-2.log`) shows the
same shape: it re-derives the plan correctly in prose ("Add imports... Create
a grammar sampler... Parse the model output...") but then hits **whitespace-
sensitive edit failures** and pivots to "rewrite the entire file", which is
exactly the failure mode specs 35/36's own guardrails try to prevent
("make SMALL, PRECISE, SURGICAL edits... do NOT rewrite the file"). This is a
**tool-call/edit-precision failure**, not a comprehension failure — granite's
prose plan was correct both times.

### 2b. Full spec 36, laguna, one-shot — **failed (context window exceeded)**

`_tmp/run-36-laguna-tool-calls.err`:

```
Error: Provider error: llamacpp: OpenAI API error (HTTP 400):
{"error":{"code":400,"message":"request (33490 tokens) exceeds the available
context size (32768 tokens), try increasing it","n_ctx":32768,
"n_prompt_tokens":33490,"type":"exceed_context_size_error"}}
```

The router's `llama-models.ini` sets `c = 32768` for both models. A single
agentic turn that includes the system prompt + tool schemas + the spec text +
a 352-line source file already exceeds that by ~700 tokens before the model
emits a single token. This is a **hard, deterministic ceiling** — not a
tuning problem, a budget problem: reading one file this size plus the tool
scaffolding pi sends is enough to blow it.

### 2c. Micro-split spec 36-part1 (new file, no existing-file read), laguna — **succeeded**

`_tmp/brief-36-laguna-micro.md` → `_tmp/run-local-laguna-xs-2.1-20260911-013135.log`
→ committed as `a37ac8a`. This micro-spec deliberately avoided the context
trap: it asked for a **new** file (`llamacpp_tools.rs`, two pure functions +
4 unit tests) and explicitly forbade reading the large `llamacpp.rs`
("Do NOT edit or read `crates/pii-model/src/llamacpp.rs` — that is a later
part"). Laguna's own transcript:

```
I'll implement the `llamacpp_tools.rs` helper functions following the TDD
contract... Now I understand the structure. Let me create the
`llamacpp_tools.rs` file... All 4 tests pass. Now let me verify the exit
criteria... Let me also verify the code compiles cleanly with clippy...
Let me fix the clippy warnings...
```

It self-corrected clippy warnings once, but not completely — see 2e.

### 2d. Micro-split spec 36-part2 (surgical edit to an existing file), laguna — **succeeded**

`_tmp/run-local-laguna-xs-2.1-20260911-015643.log`. This is the harder case
(editing the same 352-line `llamacpp.rs` that blew the context budget in 2b)
made tractable by **the spec doing the diff-finding work up front** —
`36-part2-integrate.md` gives the exact before/after code blocks for each of
4 edits, rather than asking the model to derive them. Laguna followed the
literal instructions ("apply the four surgical edits") and finished cleanly:
"Now let me verify the changes compile and run the tests."

### 2e. What still needed a human/Sonnet pass

Commit `5c9cc74` ("orchestrator polish of laguna's tests") shows the one gap:
laguna's tests used `.unwrap()` / `result[0]`, which fails this repo's
project-specific clippy lint config (`clippy::unwrap_used`,
`clippy::expect_used`, `clippy::panic` denied outside an explicit
`#![allow(...)]` in test modules). Laguna's own pre-commit clippy pass
(visible in the transcript: "Let me fix the clippy warnings... Let me fix the
clippy warnings") did not find or know about this project convention — it
needed a second, external clippy run against the *actual* configured lints to
catch it. The fix itself was mechanical (`.unwrap()` → `.expect("...")`,
`result[0]` → `result.first().expect(...)`, add the module-level `#![allow]`).

### Failure-mode summary

| Failure mode | Seen in | Root cause | Mitigation |
|---|---|---|---|
| Empty `content`, reasoning burns budget | 004, AGENTS.md (also true of cloud glm/deepseek) | thinking ON by default | `reasoning = off` in router preset — already applied |
| Context window exceeded (HTTP 400) | 2b (laguna, full spec 36) | spec text + existing large file + tool schema > `c=32768` | keep specs small enough that spec + touched-file(s) fit; split large specs (spec-split skill exists for exactly this) |
| Hallucinated dependency / import | 2a (granite) | model invents a plausible-sounding crate under context pressure | smaller diff surface (micro-spec) reduces the chance; a human/Sonnet review catches it — this is what quality-gate + orchestrator review is for |
| Whitespace-sensitive edit failure → "rewrite the whole file" | 2a (granite) | local models are worse at surgical string-match edits than at "write a new file" | spec-author should prefer new-file specs for local workhorses; when an edit is unavoidable, give literal before/after code blocks (as 36-part2 did) rather than prose instructions |
| Misses project-local lint config | 2e (laguna) | local model has no way to discover `clippy.toml`/deny-lint conventions from a narrow micro-spec prompt | quality-gate (`cargo clippy --all-targets -- -D warnings`) already catches this mechanically — treat it as a normal SDD iteration, not a special local-model case |
| Slow, more agentic turns needed | `023` §8, `0e68f42` | ~13–29 tok/s CPU vs. cloud | `--max-tool-iterations 120` (raised from default 50); budget more wall-clock, not more human intervention |

**Net**: granite/laguna are not reliable on a full, real-sized spec in one
shot (both concrete attempts against spec 36 failed, for two different
reasons — context ceiling and edit precision). They **are** reliable on a
spec sized to fit the model's context budget and, for existing-file edits,
sized to a small number of literal, pre-derived edits. This is not a new
constraint invented for local models — `spec-split` already exists in this
project's skill set for "spec too large for one workhorse pass"; local models
just have a much lower threshold for triggering it (32K tokens including the
touched file, vs. glm-5.3-flash's 1M-token window).

## 3. Test harness: wiring a local model into `pidag sdd`

### 3.1 What must be running

1. The llama.cpp **router**, exactly as in
   `_docs/02-guides/pi-local-llamacpp.md`:
   ```bash
   llama-server --models-preset /projects/_tmp/llama-models.ini \
     --host 0.0.0.0 --port 8082 --threads 16 --models-max 2 --jinja
   ```
   Verify: `curl -s http://localhost:8082/v1/models | jq -r '.data[].id'`
   must list `granite-4.2-3b` and/or `laguna-xs-2.1`.

2. `/root/.pi/agent/models.json` must carry the `llamacpp` provider override
   (`baseUrl: "http://localhost:8082/v1"`, keyless) with both model ids
   listed with `contextWindow: 32768` — this file is confirmed to exist with
   this shape per `023` (this research agent could not re-read it directly:
   it is outside the sandboxed `/projects/pi-improved` root for this session,
   so treat "confirmed per doc 023" as the evidence, not a fresh read).

3. `/root/.pi/agent/settings.json` `enabled_models` must include
   `"llamacpp/*"` (same caveat — documented in 023, not re-read here).

### 3.2 Wiring it as the `pidag` worker

`.pidag/config.toml` today:

```toml
[worker]
default_model = "ollama-cloud/glm-5.3-flash:cloud"
timeout_secs = 600
```

The AGENTS.md note that "slash-first form is REQUIRED" (pidag previously
split on `:` first, breaking `:cloud` suffixes) implies the field is a
`<provider>/<model>` string passed straight through to pi's `--provider`/
`--model`. By the same pattern, a local-model worker entry would read:

```toml
[worker]
default_model = "llamacpp/granite-4.2-3b"   # or "llamacpp/laguna-xs-2.1"
timeout_secs = 1800   # raise well above the cloud default — CPU tok/s is ~15-29,
                       # and local models take more turns (see --max-tool-iterations 120)
```

**Caveat, stated plainly**: this repo has no recorded run of `pidag sdd`
itself dispatching to `llamacpp/*` — all granite/laguna evidence in §2 came
from **direct `pi --provider llamacpp --model ... -p "..."` invocations**
(a hand-rolled harness, referred to in commit messages as
"`run-local-model.sh`", not found as a checked-in script in this tree — it
may be ephemeral/uncommitted). Before trusting `pidag sdd --run` against a
local model:

- Confirm pidag's request builder forwards `--max-tool-iterations` (or
  whatever pidag calls it) at 120+ for local workers — the cloud path needed
  an analogous explicit `reasoning_effort="none"` override because CLI flags
  don't always propagate (AGENTS.md's `--thinking off` warning); the same
  class of gap could exist for iteration caps or context-window declarations.
- Confirm pidag's `timeout_secs` is set high enough — 600s (the current
  default) may be too tight; ds4-bench B4 (3-step autonomy) took granite
  11.9s and laguna 206s for a *trivial* task (`023` §6). A real spec
  implementation loop (read → write → test → fix) at ~15-29 tok/s CPU could
  plausibly take 10-30 minutes for a single iteration.
- `[sdd] max_iterations = 4` — each failed quality-gate/validate cycle costs
  a full local-model turn; at CPU speed this could make a 4-iteration SDD
  loop take an hour-plus wall-clock for one spec. This is a cost trade-off
  (free/local vs. slow), not a correctness blocker.

### 3.3 `bundle/config.env`

No file matching `bundle/*config.env*` or `*.env` exists anywhere under
`/projects/pi-improved` (checked via `find`). If this is a deployment-bundle
concept from outside this repo's current state, it isn't present here to
wire into — flagging rather than guessing at its shape.

## 4. Adoption gate: a concrete, repeatable probe

Two gates, layered — reuse doc 004's model-level gate, then add a
spec-shaped gate this doc introduces (the thing 004 didn't test: can the
model act as an SDD workhorse, not just answer a prompt).

### Gate A — model-level (from 004, unchanged, re-confirm before trusting the router)

```bash
curl -s http://localhost:8082/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"granite-4.2-3b","max_tokens":6000,
       "messages":[{"role":"user","content":"What is 6*7? Reply with just the number."}]}' \
  | jq '{content: .choices[0].message.content, reasoning: .choices[0].message.reasoning_content}'
```
**Pass**: `content` non-empty (e.g. `"42"`), `reasoning` is `null`/absent.
**Fail**: `content: ""` with a populated `reasoning` field — the router
preset's `reasoning = off` isn't taking effect; do not proceed to Gate B.

### Gate B — spec-shaped, sized to the model's context budget (new)

A trivial spec deliberately shaped like a real micro-spec — new file, no
existing-file read, explicit TDD contract, machine-checked exit criterion —
run through the *actual* invocation path (agentic, with tools), not a bare
chat completion:

```bash
cat > /projects/_tmp/gate-b-spec.md <<'EOF'
# gate-b: trivial add function

Create `crates/pii-model/src/gate_probe.rs` with:
`pub fn add_three(a: i32, b: i32, c: i32) -> i32` returning `a + b + c`.
Add one test `add_three_sums_all_inputs` asserting `add_three(1, 2, 3) == 6`.
Declare `pub mod gate_probe;` in `crates/pii-model/src/lib.rs`.
Do not touch any other file. Do not commit.
EOF

pi --provider llamacpp --model granite-4.2-3b \
   -p "$(cat /projects/_tmp/gate-b-spec.md)" \
   --tools read,write,edit,bash,grep,find,ls \
   --max-tool-iterations 60

cargo test -p pii-model gate_probe 2>&1 | grep -qE "test result: ok\. [1-9]" \
  && echo "GATE B: PASS" || echo "GATE B: FAIL"
```

**Pass criteria** (all four):
1. `cargo test -p pii-model gate_probe` shows `test result: ok. 1 ...` (the
   test exists, compiles, and passes — not vacuous).
2. `cargo clippy -p pii-model --all-targets -- -D warnings` is clean on the
   new file (checks whether the model respects this repo's deny-lints
   convention, per §2e — expect a first-pass miss here even on success; that
   is normal, not a gate failure, as long as quality-gate catches it).
3. No file outside `gate_probe.rs`/`lib.rs` was touched
   (`git status --porcelain` shows exactly those two).
4. The run completed inside the iteration/timeout budget without a context-
   window HTTP 400 (§2b's failure mode) — if it hits that error on a spec
   this small, the router's `c=32768` or the model's usable headroom is
   unexpectedly exhausted (e.g. leftover context from a prior long session)
   and needs investigating before any real spec is attempted.

**Then, only after Gate B passes**, size-test with something closer to a
real spec: rerun the same probe but pointed at editing an *existing*
~300-400 line file (mirroring 2d's success and 2b's failure), to find the
actual file-size ceiling on this box before picking spec-split's target
chunk size for local-model runs.

Clean up: `rm -f /projects/_tmp/gate-b-spec.md` and `git checkout --
crates/pii-model/src/gate_probe.rs crates/pii-model/src/lib.rs` (or `git
clean` the new file) after recording the result — do not leave probe
artifacts in the tree.

## 5. Recommendation

**Granite-4.2-3b, not laguna-xs-2.1, as the default local-workhorse
candidate — and only for micro-specs, not full-size specs, until context
headroom improves.**

Reasoning:

1. **Speed favors granite for iteration-heavy SDD loops.** `023` §6:
   granite 12.88 tok/s vs. laguna 13.32 tok/s (roughly tied on raw speed),
   but B4 (3-step autonomy) took granite 11.9s vs. laguna 206s — laguna is
   dramatically slower on multi-step tasks despite similar raw tok/s, which
   matters directly for an SDD loop (`implement` → `quality-gate` →
   `validate`, each a multi-step tool-using turn).
2. **Both models are context-bound at 32K**, and this is the dominant risk
   (§2b) — not model capability. Granite's smaller size (2.2GB vs. laguna's
   20GB) means it loads faster and leaves more host RAM for concurrent
   `--concurrency N` workers, which matters more than raw tok/s once specs
   are correctly sized.
3. **License**: granite is Apache-2.0, IBM-verified (004). Laguna's license
   is not established in any doc read here — flag before any real adoption,
   same as 004 flagged for ornith-1.5.
4. **Conditions for use, concretely**:
   - **Spec size**: new-file specs under ~150 lines of generated code, or
     existing-file edits expressed as ≤4 literal, pre-derived before/after
     blocks (spec-author writing *for* a local workhorse, not just splitting
     for size) — mirroring 36-part1/36-part2's shape exactly.
   - **GPU vs CPU**: all evidence here is CPU-only (14-29 tok/s, per 004 and
     023). A GPU would likely change the speed trade-off enough to reconsider
     laguna or a larger granite tier (8b), but that has not been probed on
     this box — do not assume it without re-running Gate A/B on GPU hardware.
   - **Iteration budget**: `--max-tool-iterations 120`+ and a `pidag`
     `timeout_secs` well above the 600s cloud default (§3.2).
   - **Human/Sonnet-reviewed quality-gate stays mandatory** — §2e shows even
     a successful local-model run left a clippy-convention gap; this is
     exactly what the existing `quality-gate.sh` + orchestrator-review step
     is for, and nothing here suggests removing that step for local runs.
5. **Not yet recommended**: routing a real numbered spec (like 31 or 36's
   *full* form) straight to `pidag sdd` with `llamacpp/granite-4.2-3b` as
   `[worker].default_model`. Run Gate A + Gate B first, then a handful of
   real micro-specs through the actual `pidag sdd` path (not the hand-rolled
   `pi --provider llamacpp ...` harness used for all evidence in this doc) to
   confirm the iteration-cap/timeout forwarding gap in §3.2 doesn't silently
   degrade behavior the way `pi --thinking off` silently failed to forward
   `reasoning_effort` for the cloud workhorse (AGENTS.md).
