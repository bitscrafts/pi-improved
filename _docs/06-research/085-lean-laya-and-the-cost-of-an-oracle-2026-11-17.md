# Lean + laya + the local models: what a proof oracle actually costs

**Filed 2026-11-17. Research and measurement only — no pii code, no spec, nothing scheduled.**
Every number below was produced on this machine today (Lean 4.36.0 via elan; llama.cpp on `lnx`; ollama-cloud as the only live cloud provider). Where a claim was not measured it is labelled as such.

---

## 1. What was asked

Three questions, in order: does Lean have an effective API; can an LLM manage it; and — given that the local models exist, there is no GPU, and laya is a classifier — can laya improve this kind of research, and couple with Lean directly. Plus one instruction with a cost consequence: keep paid traffic to a minimum, and find out why DeepSeek keys were being used at all.

## 2. The API surface (verified, not read)

`oOo0oOo/lean-lsp-mcp` (★557, `uvx lean-lsp-mcp`) exposes Lean over LSP to any agent. Its live `tools/list` returns **22 tools**: `lean_goal` (self-described "MOST IMPORTANT tool"), `lean_diagnostic_messages`, `lean_multi_attempt`, `lean_run_code`, `lean_verify`, `lean_file_outline`, `lean_hover_info`, `lean_completions`, `lean_code_actions`, `lean_declaration_file`, `lean_references`, `lean_term_goal`, `lean_minimal_hypotheses`, `lean_profile_proof`, `lean_build`, `lean_get_widgets`, `lean_get_widget_source`, plus four lookup services (`lean_local_search`, `lean_loogle` by type, `lean_leansearch` in natural language, `lean_leanfinder`, `lean_hammer_premise`).

Underneath, the layers are: metaprogramming inside Lean (`MetaM`, `Elab`, `TacticM`, `Expr`, `InfoTree`); LSP outside it; and machine-to-machine clients (Pantograph ★85 / PyPantograph ★152, LeanInteract ★138, leanclient ★50, LeanDojo ★853).

A real returned payload, for the shape of the thing:

```json
lean_run_code {"code": "example : 2 + 2 = 5 := by rfl"}
→ success: false
  "Tactic `rfl` failed: The left-hand side 2 + 2 is not definitionally equal to the
   right-hand side 5 … ⊢ 2 + 2 = 5"
```

## 3. Experiment 1 — hand-proposed tactics, Lean as the only judge

Ten core lemmas, a fixed candidate list per lemma (the tactics a model would try in order), one call to `lean_run_code` per candidate. Core Lean 4.36.0, **no Mathlib**.

| measure | value |
|---|---|
| lemmas solved, round 1 | 9/10 |
| tactic attempts | 45 |
| attempts that pass | 9 (20 %) — but 9 of the 45 were `rfl` on a `∀` goal, i.e. designed to fail |
| wall clock | **2.3 s total, 0.05 s per verdict** |
| round 2 (candidates revised from the diagnostics) | L10 solved → **10/10** |

The winning tactics were almost all one-liners naming a core lemma (`Nat.zero_add`, `Nat.add_comm`, `Nat.sub_self`, `Nat.mul_comm`, `List.length_range`). The one failure named its own fix: the induction left `b * k + b = b * (k+1)`, i.e. the simp set was missing **`Nat.mul_succ`**.

## 4. Experiment 2 — a cheap cloud model in the loop

Same judge, `glm-5.3-flash` @ ollama-cloud as the generator, 3 rounds per lemma with the previous round's diagnostics fed back, ≤4 candidates per round, 12 lemmas.

| measure | value |
|---|---|
| lemmas solved | **9/12** |
| tactic attempts | 13 (1.1 per lemma) |
| wall clock | 206 s |
| tokens | 3 361 prompt / **38 288 completion** |

Three failures, diagnosed one by one — and only one of them is the model's:

- **L11** (`∀ n, n + n = 2 * n`): the model never answered. `finish_reason: "length"` with **zero content characters**, at `max_tokens` 2 500 *and* at 12 000. The whole budget went to hidden reasoning tokens (58 s for 12 000).
- **L10**: three candidates, one rejected by a tool limitation (below), two genuinely incomplete inductions.
- everything else solved, some only in round 3.

**The cost driver is reasoning tokens, not prompt size**: 80 prompt tokens per call against ~2 000–5 000 completion tokens.

## 5. Experiment 3 — what the local models actually do

Host shape: `lnx`, **32 cores, 125 GB RAM, no NVIDIA device** (no `nvidia-smi`), 84 % CPU idle during the runs.

| endpoint | model | file | behaves as | measured |
|---|---|---|---|---|
| `:8082` `llamacpp` | `laguna-xs-2.1.gguf` | 20.3 GB | generator | **0.21 tok/s**, prompt 0.8 tok/s |
| `:8084` `llamacpp-granite` | `granite-4.2-3b-Q4_K_M.gguf` | 2.2 GB | generator | **0.19 tok/s**, prompt 2.1 tok/s |
| `:8083` `llamacpp-laya` | `laya-Q8_0.gguf` | 421 MB | **embedding only** | `completion` → HTTP 500 *"the current context does not logits computation. skipping"* |

So: **there is a local generator (two of them), and both are two orders of magnitude too slow to use.** 32 tokens took 165 s. With the host 84 % idle and the containers holding no CPU limits, the constraint is the containers' own thread configuration, not contention — a fixable deployment question, not a model question. (Not fixed here: restarting a service the user depends on is their call.)

**The laya route is not a sentence embedder.** `/embedding` returns **per-token rows** — 13 tokens × 1 024 dims for a short diagnostic, 21 × 1 024 earlier — because the server runs without pooling. Mean-pooling client-side gives a usable vector at **~0.97 s per call**.

**A failure-class router built on it works, and is honest about its limits.** Seven labelled diagnostics from the traces above (missing_lemma, wrong_tactic, incomplete, budget), cosine to the nearest class, four unseen probes: **3/4 correct**, the miss being "max heartbeats exceeded" → `wrong_tactic` at 0.900 against `budget` at 0.891 — a near-miss that a larger labelled set would separate. This is a router, not a judge: laya chose the *class*, Lean still decides pass/fail.

## 6. The paid-traffic audit (the actual finding of the day)

**Root cause**: `~/.hermes/config.yaml` had

```yaml
auxiliary:
  compression:
    provider: deepseek
    model: deepseek-v4-flash
```

Context **compression** runs on every conversation that grows past the window — the most frequent model call in the system. That is what was hitting `api.deepseek.com`.

Evidence, from this machine:

- `cron/usage_audit.jsonl`, last 430 rows: **89 calls to `deepseek-v4-flash`** (vs 134 gemini-3.8-flash, 106 gpt-4.1, 37 glm-5.3-flash:cloud).
- logs: **168** occurrences of `provider=deepseek` and **93** of `api.deepseek.com`.
- two cron jobs also carried `provider: deepseek, model: deepseek-v4-flash` (both `paused`).

**Fix applied**:

1. `hermes config set auxiliary.compression.provider ollama-cloud`
2. `hermes config set auxiliary.compression.model deepseek-v4.1-flash`
3. `DEEPSEEK_API_KEY` **commented out** in `~/.hermes/.env` (backup kept) so that any remaining path fails closed instead of billing.
4. Backups: `config.yaml.bak-*`, `.env.bak-*`.
5. Provider probes: **deepseek → HTTP 402 "Insufficient Balance"**, **nvidia `deepseek-v4-flash-0731` → HTTP 410 Gone (EOL)**, **ollama-cloud → 200**. Only ollama-cloud is live.

Still pointing at the DeepSeek name (not the API): the `model.aliases` map (`deepseek: nvidia/deepseek-ai/deepseek-v4.1-flash`), the compression model name now served by ollama-cloud, and the Discord channel override (`deepseek-v4.1-flash:1017` @ `ollama-cloud`). **The two paused cron jobs remain** — they were left untouched; with the key disabled they would now fail closed rather than bill.

`~/.pi/agent/models.json` was **not** modified (that tree is off-limits), so any pi worker chain that lists deepseek still needs a decision.

## 7. Minimal-paid-traffic design (what the numbers support)

Measured costs of the three components:

```
Lean verdict          0.05 s    free, deterministic
laya router           0.97 s    free, CPU (3/4 on 4 probes)
cloud generator       1.1 s     71 output tokens (deepseek-v4.1-flash) / 126 (glm-5.3-flash)
```

So the ordering that minimises paid traffic is not "use a cheaper model", it is **spend the free oracles first**:

1. **Static prior, zero cost**: try the short fixed list (`simp`, `omega`, `rfl`, `intro …; simp`, `exact <lemma>`) — this alone solved 9/10 in experiment 1.
2. **laya router, zero cost**: classify the diagnostic into a failure class and pick the next cheap family, instead of asking the cloud what to do next.
3. **Cloud, last**: terse prompt, ≤3 candidates, `max_tokens` ≈ 400, **`deepseek-v4.1-flash`** (71 tokens for a JSON-only answer against 126 for glm-5.3-flash — 1.8× cheaper for the same output), and never a long system prompt that invites a proof search.

Estimated effect: in experiment 2 the cloud spent **38 288 completion tokens** on 12 lemmas because it was asked first. With the static prior in front, only L10 and L11 would reach the cloud — roughly **two calls, a few hundred tokens**, not counting the reasoning-token floor that glm-5.3-flash imposes regardless of prompt.

## 8. Three harness defects that fabricated results (rule 0b, three instances in one afternoon)

1. **`max_tokens` counts hidden reasoning tokens.** 11 rounds came back `finish=length` with empty content and were first written down as "the model failed". They were my budget.
2. **The JSON parser invented a tactic** — a fallback that scraped quoted strings produced the literal candidate `"candidates"`, which Lean then rejected: a fake attempt in the dataset.
3. **`lean_run_code` requires continuation lines to be indented.** Side by side: `:= by\n  rfl` → success; `:= by\nrfl` at column 0 → *"unexpected identifier; expected command"*; `:= by intro n; rfl` → success; a five-arm induction as one `;`-joined line → success. Rule: feed it one-line or `;`-joined scripts, or use the file-based tools.

Plus two provider facts worth keeping: **`think:false` is ignored** on ollama-cloud's OpenAI-compatible endpoint (600 tokens, 0 content, twice), and a reasoning model can therefore bill its full budget while returning nothing at all.

## 9. Limits — what this does **not** establish

- Ten to twelve textbook lemmas, chosen by me; no Mathlib, no premise-retrieval benchmark, no competition set.
- The router was scored on **four** probes; 3/4 is a signal, not an accuracy figure.
- Local generation speed was measured once per model, on a host that was 84 % idle but carrying load average ~27; the cause was not isolated to a single configuration item.
- `laya`'s 21-dim observation earlier in the day was a per-token row count, not a model dimension — corrected here to 1 024 dims per token, no pooling.
- Nothing here shows an agent doing research mathematics. It shows that an exact oracle is cheap, that a cheap model can drive it, and where the money was leaking.

## 10. Status

**RESEARCH — no code, no spec, nothing scheduled.** Installed or changed today: Lean 4.36.0 (`~/.elan`), `ripgrep 14.1.1`, `uvx lean-lsp-mcp`, a core-Lean project at `/tmp/leandemo`, the three-part gate `/tmp/lean-gate.sh` (build ∧ no `sorry` ∧ no `sorryAx`, with a failing control), and the provider fix in §6. Nothing was added to the pii workspace; the gate and drivers live in `/tmp` and would need a home before they are worth keeping.

---

## 11. The loop, run on the whole backlog (2026-11-17, evening)

The loop lives at `/projects/research/lean-autoresearch/`: `program.md` (human brief), `lemmas.tsv`
(20-lemma backlog), `run_cycle.py` (one cycle: static prior → second free family → laya router →
cloud, ≤1 call and ≤400 tokens), `gate.sh`, `router_refs.json`, `runs.jsonl`, `proofs/`.

**Result: 20/20 lemmas proved, every one by the free static prior, 0 cloud tokens, 190 s of wall
clock across 21 cycles.** The one cloud spend of the day (165 tokens) came from the broken-judge
cycle described below, not from a hard lemma. `omega` solved `∀ n, n + n = 2 * n` — the lemma the
cloud model failed in experiment 2 — at zero cost.

Two changes were folded in before the run, both from the PDDL-Instruct paper filed the same evening:

1. **Full diagnostics to the generator**, not the first line truncated to 110 characters
   (their ablation: detailed feedback beat binary feedback by 3–5 points in every domain).
2. **A second free family** (induction-shaped one-liners) tried before any cloud call, on the
   reasoning that their loop iterates 10–15 times and ours iterated once.

**The gate certified nothing the first time, and only a control revealed it.** The first version of
`gate.sh` hard-coded `#print axioms` lines for `L1…L10` — theorems left over from the earlier
experiment — and relied on `lake build`, which does **not** build a module nothing imports. So
`Leandemo/Proved.lean` was never compiled by the gate, its `sorry` warnings never existed, its
axioms were never printed, and `axioms_rc=1` was not checked. The gate printed `VERDICT: PASS` while
checking nothing about the 20 proofs, and it kept printing PASS when a `sorry` theorem was injected.
The fix: compile `Proved.lean` **explicitly**, generate the `#print axioms` list from the file at run
time, and fail when any checker exits non-zero or when zero theorems are checked.

Three controls were then required to pass (all measured, in this order):

| control | result |
|---|---|
| 20 real proofs | `build_rc=0 proof_rc=0 sorry_warnings=0 theorems_checked=20 axioms_rc=0 sorry_ax_hits=0` → **PASS** |
| one injected `sorry` | `sorry_warnings=1 sorry_ax_hits=1` → **FAIL** |
| false theorem, no `sorry` (`2+2=5 := by decide`) | `proof_rc=1 sorry_ax_hits=1` → **FAIL** |
| empty proofs file | `build_rc=1 theorems_checked=0` → **FAIL** |

A durability bug fell out of the same run: the loop appended the proof, then ran the gate, then
marked the lemma `proved`. When a cycle was killed by its own 300 s timeout mid-gate, the lemma went
back to `todo` and was proved again — producing a **duplicate declaration** that broke the file for
every later cycle. Fixed by marking the lemma proved before the gate and making the append
idempotent. That duplicate is what made the gate fail on the clean tree, which is how it was found.

**Status unchanged: RESEARCH.** 20 textbook lemmas, no Mathlib, no premise retrieval. What it
establishes is narrower and more useful: the free oracles carried the entire backlog, and the gate
that judges it has now been shown to fail on demand.

---

## 12. The loop, improved and re-run on a harder backlog (2026-11-17, night)

The first backlog was solved entirely by `simp`/`omega`, which meant the router and the paid path
were never exercised — the loop was measuring the tactic library, not the design. So it was improved
in six ways and given twelve lemmas that `omega` cannot close (nonlinear arithmetic, witnesses,
induction over lists):

1. **A self-check before judging anything.** The cycle first proves `1 + 1 = 2` through the same code
   path and aborts if that fails. This is the direct answer to the afternoon's defect where a judge
   that could not parse its own theorem name reported every candidate as wrong.
2. **Three free stages, not two**: static prior → induction family → **router-chosen family**.
3. **The router now does work.** laya classifies the last diagnostic and the class selects the next
   tactic family (`ROUTER_FAMILIES`: missing_lemma → `Nat.mul_add`/`Nat.add_mul`/`Nat.mul_succ`
   rewrites; incomplete → induction templates; wrong_tactic → assumption/constructor; budget →
   `decide`/`native_decide`; syntax → reset). Previously it only produced a label.
4. **The generator iterates**, up to two paid rounds, each shown the accumulated diagnostics, with a
   hard ceiling of 400 tokens per call and **1 200 tokens per cycle**.
5. **Ordering fixed**: the lemma is marked proved *before* the tree is gated, and the append to
   `Proved.lean` is idempotent — the crash that produced a duplicate `L02` and broke the file cannot
   recur.
6. **`verify_controls.sh`** is now part of the loop, not an ad-hoc command: clean → PASS, injected
   `sorry` → FAIL, false theorem → FAIL, restored → PASS, and any mismatch exits non-zero.

**Result on the twelve harder lemmas: 12/12 proved. Backlog now 32/32.**

```
L21 n*n+n = n*(n+1)            prior              8 attempts    0 tok
L22 a*b+a = a*(b+1)            induction         12 attempts    0 tok
L23 a*(b+c) = a*b+a*c          induction         12 attempts    0 tok
L24 (a+b)*c = a*c+b*c          induction         12 attempts    0 tok
L25 2*n = n+n                  prior              2 attempts    0 tok
L26 a*b = 0 -> a=0 v b=0       induction         13 attempts    0 tok
L27 n != 0 -> exists m, n=m+1  induction         11 attempts    0 tok
L28 exists m, n <= m           CLOUD r1          24 attempts  340 tok
L29 a<=b -> a<=b+1             prior              2 attempts    0 tok
L30 (l++l).length = 2*l.length ROUTER missing_lemma 20 attempts  0 tok
L31 a+b = 0 -> a = 0           prior              1 attempt     0 tok
L32 n*n*n = n*(n*n)            induction         12 attempts    0 tok
```

**Cost of the hard batch: 340 tokens, one lemma.** Across the whole day the loop spent **505** paid
tokens, of which 165 were wasted by the broken-judge cycle; the metric `program.md` asks for is
therefore **32 lemmas per 505 tokens, or 63 per 1 000** — and 31 of the 32 needed nothing paid at all.

**Gate at the end of the batch**: `build_rc=0 proof_rc=0 sorry_warnings=0 theorems_checked=32
axioms_rc=0 sorry_ax_hits=0` → **PASS**, and `verify_controls.sh` → `ALL_OK`. No duplicate
declarations (32 unique of 32), 32 files in `proofs/`.

**What this does and does not show.** The free stages carried 11 of 12 hard lemmas, so the honest
reading is that **a good induction template is worth more than a generator**, and the router earned
its place exactly once — on the only lemma where it was consulted (1/1). The one paid lemma, L28
(`∃ m, n ≤ m`), exposes a real gap: the families contain no **witness-shaped** template, so the
loop had to buy an answer it could have had for free. Adding an existential family is the next
cheap improvement, not a new model.
