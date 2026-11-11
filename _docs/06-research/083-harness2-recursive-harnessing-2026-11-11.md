# Harness² (Google Research): the harness as the thing you optimise — and why our gate makes us a better test bed than the paper has

Filed 2026-11-11. **Evaluation only — no code, no spec, nothing scheduled.** Primary source: `google-research/harness2` (README, Apache-2.0) and arXiv:2610.14224v1, read 2026-11-11. **The framework has not been installed or run here**; every claim below is either quoted from the paper/README or marked as inference.

## 1. What it is

*Harness* = the system around the frozen model (prompts, tools, memory, skills, orchestration). *Harnessing* = optimising that system. Harness² automates it with **two coupled loops**:

- **Task-level recursion** — refine the execution harness for the *current* task with a contrastive **Propose–Probe–Compose** step: a *harness editor* proposes invariant and domain edits, candidate *probes* run them under matched conditions, a *composer* keeps changes supported by behavioural contrast. Scales as **parallel** (several candidates at once) or **sequential** (one harness refined step by step).
- **Harnessing-level recursion** — after the task, an *experience reflector* updates a persistent **improvement harness** from a **harness improvement library** (adopted harnesses + lessons about when and why they worked). It does *not* simply inherit the previous execution harness.

**Eight editable components**: system prompt · guardrails · memory · tool guidance · subagents · skills · scripts · plugins. Model weights never change.

**Release supports**: parallel and sequential recursion, **OpenCode and Codex executors**, benchmarks JobBench, LAB (Harvey Legal Agent Benchmark), WorkBuddy-Bench. Python, Apache-2.0, created 2026-10-06, ~23k stars / ~2.3k forks by 2026-11-02.

## 2. The measured result, stated as the paper states it

> "Across three professional benchmarks and four model–harness configurations, a single Harness² step improves GLM-5 and Gemini 3.5 Flash over their base harnesses, with gains of up to 11.5 points on WorkBuddy-Bench, 4.5 points on JobBench, and 11.8 percentage points on LAB's partial pass rate."

Parallel recursion adds up to 3.9 (WorkBuddy) and 4.2 (JobBench) beyond a single step. One step improved **all three benchmarks in every configuration** — that is the strongest part of the result.

## 3. What the paper itself says it costs

> "Harness² requires additional executions and editor sessions to propose, probe, and compose harness changes."

So the gain is bought with inference, not free. In our terms: **probe = run the same task more than once.** That is cheap on free/ollama tiers and expensive on paid Claude turns — which makes this a budget decision, not a toggle.

## 4. The distinction that decides how we use it

The paper's hardest constraint is composing edits **with no ground-truth verifier**: the composer decides which behavioural contrast is better, and that judgement is made by a model. That is a real limitation they are open about — and it is exactly the gap our gates exist to close.

**We are in a strictly better position for our own work**: `cargo xtask gate`, clippy `-D warnings`, criterion PASS/FAIL, spec exit criteria and CI are ground truth. Therefore:

- where a real oracle exists (build, tests, gate, criteria): **do not** copy "judge by behavioural contrast" — compose on the measured result;
- where no oracle exists (does this report read well, was this review useful, is this doc clear): the contrastive method is the only honest option, and it should be labelled as judgement, not measured.

## 5. What is worth taking, in order

1. **The harness improvement library** — the cheapest and most transferable idea. Record the *lesson* (when and why a change worked), not just the patch. Our version of this is already half-built in memory + skills; the missing half is the condition under which an edit helped.
2. **Propose–Probe–Compose as a manual discipline** — three candidate variants against one explicit PASS/FAIL criterion, with a written lesson for the adopted one. No install required.
3. **Parallel recursion only where the harness outlives the task** — the AGENTS.md layer, the gate hooks, the skill files, the pii graph. Skip it for one-offs where probe cost exceeds harness life.
4. **The eight-component checklist** — a ready list for answering "what is actually editable in pii / Hermes?" with a file per row instead of an opinion.

## 6. Limits of the evidence

- Only **GLM-5 and Gemini 3.5 Flash**; no Claude, no GPT, no open-weight mid-size model.
- "Gains of up to" = best configuration on the best benchmark, not an average.
- Benchmarks are professional-task suites (two of them legal/office-agent shaped). **Nothing here establishes transfer to code changes, hardware bring-up, or a Rust workspace with a hard gate.**
- Preprint; not peer-reviewed.

## 7. Relation to earlier notes

- `082-harness-engineering-eleven-systems` — the Wavestone study describes the *anatomy* of eleven harnesses; this paper changes the *optimisation loop* over one. Together they answer "what is a harness" and "how does it get better".
- `079-dsh-superagent-evaluation`, `081-egm-evaluation` — same family: an outside mechanism that refines the scaffolding rather than the weights.
- Our own `AGENTS.md` + hook + skill edits are this loop run by a human. The paper's contribution is automating and measuring it; ours to gain is the *lesson library* half.

## 8. Status

**RESEARCH.** No install, no spec, no cron. If it is ever tried, the experiment worth running is one recurring failure we currently fix by hand in `AGENTS.md` or a hook, with a PASS/FAIL criterion that comes from the gate — not from a model's opinion.
