<!-- MERGED-CORPUS 2026-09-28T20:00:57Z: this file is the canonical document for its subject.
     It absorbed 2 document(s) which are now sections below, kept verbatim.
     Why: both absorbed docs are measurement evidence for the same question.
     Originals archived at _docs/99-archive/06-research-merged-2026-09-28/ (nothing deleted). -->

# Local models as the SDD workhorse — feasibility, with the tier probes

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

- [004](014-local-model-spec-implementation.md) established the non-negotiable gate:
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


---

## [merged] Granite 4.2 nano/small-tier probe + flexible model coupling

<!-- Absorbed from `014-local-model-spec-implementation.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

**Date**: 2026-09-02 · **Author**: orchestrator (probe) + opus (recommendation) · **Status**: DECISION-IN-PROGRESS

## Context
The pass-3 architecture (`002-pii-architecture-plan-pass2.md`) defines:
- **Tier 0 (nano)** = route/classify/extract, one call, no reasoning, escalates; 10–100ms, $0.
  Default backend is **B1** (an `Arc<dyn Embedder>` + fitted head, pure Rust, no chat model) — pass-3 §3.1.
  Do **not** use a chat LLM at tier 0 (occupies the single Ollama slot, violates latency).
- **Tier 1 (small)** = local ~9B via Ollama/llama.cpp, ~1s: routing, promoted runs, tool-arg repair, summarisation, cheap planning — pass-3 §2 row `:175`.

## opus recommendation (2026-09-02, grounded in pass-3)
- Standardize tier 1 on **granite4.2:8b** (primary, Apache-2.0, 5.3GB/128K) + **granite4.2:3b** (same-family fast lane).
- **ornith-1.5:9b** = fallback only; its **license is UNVERIFIED** (ollama page states none) and its thinking-control is unknown.
- **No chat model at tier 0** — B1 embedder stays.
- Non-negotiable **adoption gate**: one-shot probe `max_tokens≈6000` must show content>0 AND reasoning==0, with the exact wire field recorded.

## Official IBM Granite 4.2 doc (authoritative, https://www.ibm.com/granite/docs/models/granite4-2)
- Dense reasoning family, 3B/8B/30B, **Apache 2.0** (crypto-signed, ISO certified), 128K ctx (512K on 30B).
- **Thinking modes are chat-template parameters**:
  | Mode | Template Params |
  |---|---|
  | Thinking (default) | `enable_thinking=True` — full CoT in `thinking...response` |
  | **Non-thinking** | `enable_thinking=False` — **direct answer, no CoT** |
  | Low-effort | `enable_thinking=True, low_effort=True` — brief reasoning |
- Generation params: `temperature=1.0, top_p=0.95` across all tasks/backends.
- **IBM optimizes for vLLM** (v0.20+): `vllm serve ... --reasoning-parser granite_thinking_parser --tool-call-parser qwen3_coder --enable-auto-tool-choice`, exposes OpenAI-compatible API. Also integrates with agentic harnesses (OpenCode, Pi, OpenHands) out of the box.
- This confirms llama.cpp `--reasoning off` == `enable_thinking=False` (Non-thinking mode).

## Probe (verified on THIS no-GPU box, via llama.cpp)
Model: `granite-4.2-3b-Q4_K_M.gguf` (2.24GB) from `ibm-granite/granite-4.2-3b-GGUF` on HF.
Server: `llama-server -m ... -c 4096 --reasoning off` (llama.cpp server build `a1f96d4`).

| Config | Result |
|---|---|
| Default (thinking ON) | `content: ''`, `reasoning_content` burns budget, `finish: length` — **same empty-content trap as glm-5.3** |
| `--reasoning off` (== Non-thinking) | `content: '42'`, `reasoning_content: NONE`, `finish: stop`, ~28 tok/s; JSON output clean |

**Conclusion**: granite-4.2 is CPU-viable for tier 1 and passes the thinking-off gate, **but ONLY when served with `--reasoning off`** (llama.cpp) or `enable_thinking=False` (transformers/vLLM). Notes:
- The modern llama-server flag is `--reasoning off` (`-rea`). The older `--chat-template-kwargs '{"enable_thinking":false}'` is now **deprecated** (server warns "Use --reasoning on/off instead"). Body-level `enable_thinking:false` / `reasoning_effort` are **not honored** by this llama.cpp server build.
- On Ollama, reasoning is NOT controllable at all (Ollama replaces the model's own Jinja template that carries the toggle) — so granite tier-1 must run via llama.cpp or vLLM, not Ollama.
- CPU speed ~14–28 tok/s on 32 cores / no GPU. Fine for tier-1 ~1s latency budget on short prompts.

## Flexible model coupling (user directive, 2026-09-02)
> "when attaching to pii, make the coupling flexible, allowing the owner to attach any model (if possible and as future work)"

**Design intent**: model backends MUST be pluggable — a driver trait / factory the owner configures (`model[provider]`-style, mirroring how the worker chain already picks providers), NEVER hardwired to one vendor. Concrete consequences:
- A `ModelProvider`-style trait (sync `chat(&Ctx, &ChatRequest) -> ChatResponse`) already anticipated in the Phase-2 plan (`pii-model`); keep it the single seam so an owner can attach granite/llama.cpp/vLLM, ornith/GGUF, ollama-cloud, or any OpenAI-compatible endpoint. IBM's own doc confirms granite's OpenAI-compatible API "integrates with ... Pi" — so the seam is a standard one.
- Keep the nano ROLE (route/classify/extract) decoupled from any specific model — a `NanoRouter` impl can be embedder-backed (B1, default) OR an LLM-backed driver attached later (B2′). Tier 0 stays B1 by default.
- This is "attach any model" as future work; record here so later phases keep the seam open.

## Actionable next steps (not yet done)
- [ ] Save this as the canonical decision doc; link from `_docs/INDEX.md` / `TOC.md`.
- [ ] In the Phase-2 `pii-model` spec, keep `ModelProvider` the single pluggable seam (per above).
- [ ] Keep `granite-4.2-3b-Q4_K_M.gguf` at `/projects/_tmp/` as the verified tier-1 candidate; note in `AGENTS.md`/models config that tier-1 must run via llama.cpp `--reasoning off`, not Ollama.
- [ ] `ornith-1.5` license + thinking-control: verify before any adoption.


---

## [merged] Tiny chat models (Needle and similars) — real usage inside Pii

<!-- Absorbed from `014-local-model-spec-implementation.md` on 2026-09-28T20:00:57Z during the SSOT corpus merge.
     Content preserved verbatim. Original moved to 99-archive/06-research-merged-2026-09-28/. -->

**Date**: 2026-09-10 · **Author**: sonnet (research) · **Status**: RESEARCH — no decision, no code touched

## 0. Scope and method

This revisits the "tiny models" question raised and partly settled by
[`014-local-model-spec-implementation.md`](014-local-model-spec-implementation.md) and
[`002-pii-architecture-plan-pass2.md`](002-pii-architecture-plan-pass2.md) §1/§3. That prior
work already rejected a tiny **chat** model at tier 0 in favor of a deterministic embedder
(B1) and picked granite-4.2-3b for tier 1. This doc asks the narrower question: is there
*any* real spot inside Pii — not necessarily tier 0 — where a 0.5B–3B **chat** model earns
its keep over (a) the existing deterministic/embedder path or (b) just calling tier-1
granite?

Method: read the prior decision docs and the actual current code (not just the plan) to see
what's implemented vs. still paper, then reason about each candidate use case against that
ground truth. External model facts (§2) are drawn from training knowledge, not re-verified
against vendor pages in this session (no live web access was available) — treat the specs
table as a starting point to re-verify before any adoption, the same way 004 insisted on a
live CPU probe before trusting a spec sheet.

`_docs/06-research/016-ml-applications.md` does not exist in this repo — there is no prior
ML-applications doc to cross-reference.

## 1. What Pii actually has today (ground truth, not plan)

This matters because the prior docs (pass 3, 004) describe a **target** architecture with a
`nano` node kind, a `NanoRouter` trait, and B1/B2/B3 backends. None of that exists in code
yet:

- `crates/pii-graph/src/node.rs` registers exactly five built-in node kinds:
  `transform`, `wait`, `emit`, `route`, `timer`. `pii-server` adds `agent` (`agent.rs`) and
  `tool` (`tool.rs`). **There is no `nano` kind, no `NanoRouter` trait, no `pii-nano` crate.**
  `grep -rn "NanoRouter\|kind.*Nano"` across `crates/` returns nothing.
- The single model in the system today is whatever `ModelProvider` the daemon config
  resolves — in the shipped default (`crates/pii-server/src/daemon/config.rs:388`),
  `model = "ollama:granite4.2:3b"`. Every `agent` node run goes through this one provider
  (`crates/pii-server/src/agent.rs:26,42,263`). There is no tier split in the runtime: one
  model does routing, planning, tool calls, and repair alike.
- The "B1 embedder" pass 3 speced (`Arc<dyn Embedder>` for memory, doubling as the nano
  router's feature extractor) is `TrigramEmbedder` in
  `crates/pii-memory/src/embed.rs:34` — a **pure-Rust, non-learned, deterministic** 256-dim
  character-trigram hash embedder (FNV-1a into buckets, `sqrt(tf)`, L2-normalized). It is not
  a model at all, let alone a tiny model. `OllamaEmbedder` — the one alternate embedder
  backend pass 3 discussed and flagged as a coupling risk ("Tier 0 works offline is false in
  the default config if B1 needs Ollama") — **was deleted** in the phase4.7 cleanup
  (`72d1aa2 fix(phase4.7): residual defects B1/B4/B5 - delete OllamaEmbedder, ...`). So that
  coupling risk pass 3 flagged is now moot: memory embedding is 100% offline, 100%
  non-ML, by construction. This is worth stating plainly because it changes the tiny-model
  question — there is currently *no* embedding-quality gap a tiny model could close either;
  `TrigramEmbedder` was a deliberate choice, not a placeholder.
- There is no tool-output summarization node. `agent.rs` has a hard `max_tokens` cap
  (`crates/pii-server/src/agent.rs:237,291`) that errors out ("max_tokens exceeded") rather
  than compressing — a real, currently-unhandled gap (see §3.3).
- `pii-model`'s `ModelProvider` trait (`crates/pii-model/src/lib.rs:118`) is the one seam
  that matters for any of this: `fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError>`,
  sync, provider-agnostic. Anything — Ollama, llama.cpp HTTP, a future in-process GGUF
  runner — plugs in here without touching `pii-graph` or `pii-server`. This is the correct,
  already-built seam for wiring in a tiny model; no new abstraction is needed.

**Bottom line**: "tiny model real usage" in Pii today is not a tuning question on an existing
nano tier — it's a green-field decision, because the nano tier doesn't exist yet and the one
model in the loop (granite-4.2-3b) is currently doing tier-0-shaped jobs (routing, quick
classification) by brute force, at tier-1 latency (~1s+, per the 004 probe: 14–28 tok/s CPU),
because there is nothing cheaper installed.

## 2. Tiny chat model landscape (2026)

Sizes/specs below are from training knowledge (last verified against vendor docs before this
session's knowledge cutoff); the granite-4.2-3b row is the exception — those numbers are
**live-verified on this repo's own CPU box**, per 004. Re-probe anything else the same way
before trusting it for a latency or license decision.

| Model | Params | License | GGUF? | Ctx window | CPU tok/s (rough, no-GPU, Q4) | Instruction-following |
|---|---|---|---|---|---|---|
| **Needle (Cactus Compute)** | ~45M (per pass 3 §1.3: "a 45M model") | Apache-2.0 (model); **engine redistribution terms unread/unstated** | **No** — CQ2-bit weights baked into Cactus's own C++ engine; no GGUF, no Ollama tag, no llama.cpp path | Unstated in prior research; likely small (mobile-NLU scale) | N/A — not llama.cpp-compatible; own engine, prebuilt binary fetched from HF | Weak by design — built for closed-set NLU (intent/slot), not open chat |
| **granite-4.2-3b** | 3B (dense) | Apache-2.0 (verified: crypto-signed, ISO cert) | Yes — `ibm-granite/granite-4.2-3b-GGUF` | 128K | **14–28 tok/s, live-probed** on this box (`--reasoning off`) | Good — IBM-tuned for agentic/tool-call use; needs the thinking-mode gate (004 §"Probe") or content comes back empty |
| **Qwen2.5-0.5B / 1.5B** | 0.5B / 1.5B | Apache-2.0 | Yes, widely (Qwen team + community quants) | Up to 32K (native config; quality degrades hard near the top at these sizes) | 0.5B: ~60–100 tok/s; 1.5B: ~25–40 tok/s (extrapolated from the granite-3B probe's ~20 tok/s at 3B — roughly inverse to param count on the same box) | Best-in-class for size on instruction-following/JSON/function-calling benchmarks; the strongest sub-2B chat model as of early 2026 |
| **SmolLM2** (135M/360M/1.7B) | 135M–1.7B | Apache-2.0 | Yes (HF ships GGUF quants directly) | 2K–8K depending on variant | 135M: 100+ tok/s; 1.7B: ~15–25 tok/s | 1.7B-Instruct is usable for simple structured tasks; 135M/360M are closer to completion models — thin instruction-following, prone to schema drift |
| **Phi-3-mini** | 3.8B | MIT | Yes | 4K (`-4k-instruct`) or 128K (`-128k-instruct`) variant | ~10–20 tok/s | Strong for size — curated "textbook-quality" training data, above-average structured output and light reasoning |
| **TinyLlama-1.1B** | 1.1B | Apache-2.0 | Yes | 2K (extendable) | ~40–80 tok/s | Weak — largely superseded by Qwen2.5-0.5B/1.5B and SmolLM2 for anything needing schema-constrained output; treat as legacy |

Two observations that matter more than any single row:

1. **Needle is not a drop-in candidate for anything in this table.** It's not a general chat
   model — Cactus built it for closed-set on-device NLU (this matches pass 3's own framing:
   "the nano LLM's advantage is free-form NL → arbitrary schema-valid args", i.e. extraction,
   not conversation). And it fails Pii's own build-dependency bar from pass 3 §3 — no GGUF,
   prebuilt binary fetched at runtime from HuggingFace, Python-only bindings, musl/ARM64
   support unstated. That verdict hasn't changed; nothing in this research surfaces a reason
   to revisit it.
2. **Qwen2.5-1.5B is the only genuinely new option** relative to what 004 already
   evaluated (which only looked at granite-4.2-3b and ornith-1.5:9b for tier 1, and B1/B2/B3
   for tier 0). It's the model actually worth a probe if this doc's recommendation (§4) is
   pursued.

## 3. Where a tiny chat model could plug in — evaluated against Pii's real shape

Pass 3 §1.5 already established the load-bearing principle: **a nano/tiny model must never
be the last decision before a `MUTATES_EXTERNAL` effect**, and it loses to a deterministic
check whenever one is available (§1.5's example: exit code + regex beats a nano `classify`
call for "did the test pass"). Apply that filter to each candidate:

### 3.1 Prompt routing / classification ("is this a coding task or a question?")
Pass 3 already answered this in detail (§3.1): a fitted classifier over `TrigramEmbedder`
features (B1) wins over any LLM here — deterministic, replayable from the journal,
calibrated confidence, zero new deps, improves via `pii nano tune`. A tiny chat model adds
non-determinism and an extra process/IPC hop for a closed-set decision a linear classifier
already solves better. **No change to that verdict.** A tiny chat model is not the tool for
this job, full stop — this was already the right call.

### 3.2 Tool-argument repair (fix malformed JSON args)
This is genuinely closer to what a small *instruction-tuned* model is good at — it's
schema-constrained (you have the target JSON schema right there), short input/output, and
"free-form NL → arbitrary schema-valid args" is exactly the capability pass 3 §3.1 credited
to the nano-LLM backend (B2) over B1. But two things cut against adopting a *tiny chat
model* specifically for it in Pii:
- **Malformed-JSON repair from a well-formed-but-broken LLM output is usually a parsing
  problem, not a language problem.** The common failure modes (trailing comma, unescaped
  quote, truncated at `max_tokens`, wrapped in markdown fences) are handled correctly and
  deterministically by a tolerant JSON5/JSONC parser or a bracket-balancing repair pass —
  no model call needed, 0ms, no failure mode of its own.
  Only the harder case — the model genuinely produced semantically-wrong-shaped args (wrong
  field names, hallucinated keys) — needs re-reasoning about *what the tool wants*, and a
  0.5–3B model does not reliably outperform just re-asking tier-1 granite-4.2-3b with the
  schema and the error appended, at a cost of maybe 200–400ms extra on an already-degraded
  path. Given repair is already rare (pass 3 §1.5 point 3: `repair_attempts: 0` in the
  existing design), this isn't a hot enough path to justify a second model in the process.
- **Verdict: deterministic JSON-repair pass first (cheap, catches most cases); fall back to
  the already-present tier-1 provider for the rest.** No tiny model needed.

### 3.3 Summarization of long tool output before it hits context window
This is the strongest real candidate, because it's a **currently-unhandled gap**, not a
displaced alternative — `agent.rs` today just errors on `max_tokens exceeded`
(`crates/pii-server/src/agent.rs:291`) rather than compressing. Properties that favor a tiny
model here:
- The task is genuinely generative (summarize N KB of shell/test output into a paragraph),
  which a deterministic heuristic (head/tail truncation, grep-for-errors) does *worse* than
  even a weak LLM — truncation silently drops the one error line that mattered.
- It's off the critical decision path — a summarization node's output feeds *back into* the
  next tier-1 reasoning turn; it doesn't itself gate a `MUTATES_EXTERNAL` effect, so pass 3's
  §1.5 "never terminal" rule isn't violated.
- Latency budget is generous (this happens once per oversized tool call, not per token), so
  even Phi-3-mini or granite-4.2-3b's ~15-20 tok/s is tolerable — but a 0.5–1.5B model at
  40-80 tok/s turns a ~2s summarization pause into ~0.5s, which matters if this fires
  multiple times in one agent run (e.g. a build log, then a test log, then a diff).
- Risk: summarization quality directly affects downstream reasoning correctness (an omitted
  error line silently misleads tier 1). This needs eval against real tool-output samples
  before trusting it in a production run — not a blind adopt.

**Verdict: real candidate.** Qwen2.5-1.5B or granite-4.2-3b (already installed, zero new
dependency) are both plausible; §4 recommends starting with granite-4.2-3b itself run at
short `max_tokens` rather than installing a second model, because the summarization job
doesn't need to be *fast*, it needs to not drop the wrong line, and a bigger model is safer
at that until proven otherwise.

### 3.4 Extract structured facts for memory
Same shape as tool-arg repair (schema-constrained extraction), and the same B1-vs-B2 framing
pass 3 already laid out applies: `TrigramEmbedder` (B1) cannot do free-form NL→structured
extraction (it has no generative capability at all — it's a hash embedder), so *if* Pii wants
memory writes to include extracted structured facts (not just raw text + BM25/vector
indexing, which is all `pii-memory` does today per `crates/pii-memory/src/{bm25,vector,doc}.rs`),
this is a real gap a tiny model could fill that B1 structurally cannot.
But: nothing in the current `pii-memory` crate does structured extraction at all yet — there
is no consumer for this capability in the codebase today (`consolidate.rs`/`decay.rs` operate
on existing documents/scores, not on extracting new facts from text). Building this is
speculative ahead of a concrete need. **Verdict: plausible future use, not a current gap** —
revisit when `pii-memory` grows a feature that actually wants structured facts (a "memory
consolidation" pass that turns a raw conversation into typed facts would be the trigger).

### 3.5 Gate decisions (pass/fail on test output)
Pass 3 §1.5 already covers this explicitly and correctly: "exit code plus a regex is better,
cheaper, and deterministic" than any model call, tiny or not. Test pass/fail has a ground
truth (the exit code, the assertion count) that no model should be interposed in front of —
doing so adds a hallucination surface to a decision that's already unambiguous.
**Verdict: no model, tiny or otherwise. Deterministic check only. Confirmed, not revisited.**

## 4. Recommendation

**Do not adopt Needle, or any dedicated tiny chat model, right now.** Two independent reasons:

1. **The nano tier itself doesn't exist in code yet.** Nothing in §3 needs a *second* model
   process today — the one real gap (§3.3, tool-output summarization) can be served by the
   tier-1 model already configured (`granite4.2:3b` via `ollama:`), through the exact same
   `ModelProvider` seam, by adding a `summarize` mode/prompt and a shorter `max_tokens`/
   `num_ctx` on that call. That's a few hours of work behind the existing seam, not a new
   dependency, new build target, or new process.
2. **Every candidate that *would* justify a dedicated tiny model (3.1 routing, 3.5 gating)
   is better served by what pass 3 already chose — a deterministic classifier or a plain
   exit-code check — and that verdict holds up under this second look, not just the first.**
   The one genuinely new capability a tiny *generative* model adds over B1
   (`TrigramEmbedder`) — free-form extraction — has no current consumer in `pii-memory` to
   justify building it speculatively.

**If/when a second, smaller model becomes worth installing** (e.g. summarization volume
grows enough that granite-4.2-3b's ~20 tok/s becomes the bottleneck, or `pii-memory` grows a
fact-extraction consumer), the concrete pick is **Qwen2.5-1.5B-Instruct**, not Needle and not
TinyLlama:
- GGUF-native (no vendor/artifact lock, unlike Needle's prebuilt-binary Cactus engine —
  same objection pass 3 §3 raised against Needle applies unchanged today).
- Apache-2.0, same license family already in use (granite, and matches pass 3's
  license bar).
- Best-in-class instruction-following at its size for schema-constrained tasks
  (summarization, extraction) — the two jobs §3 actually found real gaps for.
- Runs through the **same `ModelProvider` seam** already built (`crates/pii-model/src/lib.rs`)
  — either as a second `ollama:qwen2.5:1.5b` spec, or via llama.cpp HTTP exactly like the
  004 probe already validated for granite (`llama-server -m ... -c 4096 --reasoning off`,
  if Qwen2.5 needs the same thinking-mode gate — verify before adoption, don't assume).
- **Gate before adoption** (same bar 004 set for granite): a live CPU probe on this box
  confirming `content` is non-empty, no reasoning-token trap, and a real tok/s number — do
  not trust the table in §2 uncritically.

Needle specifically stays rejected for the same reason pass 3 §3 already gave: it is a
vendor-and-artifact lock (prebuilt binary fetched at runtime, no GGUF, Python-only bindings,
musl/ARM64 unstated, engine redistribution terms unread), not a model-capability problem —
and that objection is orthogonal to whether a tiny chat model has a job to do in Pii. It
doesn't have a viable integration path into a static-musl Rust binary regardless of how good
the model itself is at 45M params.

## 5. Summary table

| Candidate use | Real gap today? | Tiny chat model wins? | What wins instead |
|---|---|---|---|
| Route/classify (§3.1) | No — already B1 (deterministic) | No | `TrigramEmbedder` + fitted head (unchanged from pass 3) |
| Tool-arg JSON repair (§3.2) | Rare (repair_attempts≈0) | No | Tolerant JSON parser first; tier-1 model as fallback |
| Tool-output summarization (§3.3) | **Yes — currently errors out** | Only for latency, not correctness | Tier-1 model (granite-4.2-3b) via existing seam, new prompt mode |
| Structured fact extraction for memory (§3.4) | No current consumer | Plausible later | Not yet — no feature needs it |
| Gate pass/fail (§3.5) | No | No | Exit code + regex (unchanged from pass 3) |

**Net: zero new model processes recommended right now.** The one real gap is closeable with
the model Pii already runs, through the seam it already has.
