# Harness engineering as a measured discipline: Barbaste et al., *a source-code study of eleven systems* — and where pii sits in the sample

Filed 2026-11-10. **Evaluation only — no code, no spec, no crate touched.** Primary source: Paul Barbaste,
Tristan Darrigol, Germain Vu and Tom Wiltberger (Inclusive Brains / Wavestone AI Lab), *Harness Engineering:
Anatomy, Architecture, and Evolution of Coding Agents — A Source-Code Study of Eleven Systems*,
**arXiv:2609.00006v1 [cs.SE]**, 15 July 2026, 83 pages, 7 figures, 18 tables; second, substantially expanded
edition of an April 2026 study (corpus 8 → 11 systems plus one meta-harness contrast point), explicitly **not
peer reviewed** and explicitly **not a benchmark**.

**Read for this note:** the abstract page, the paper's own page 1 (title block, one-line algebra, the
corpus list, the two absences, the extensibility and longitudinal findings), one published reader's summary,
and the arXiv comments field. **The 83-page body has not been read**, and in particular the per-system
sections on **Pi** and **Hermes** have not. Everything below that concerns pii is therefore a *mapping to be
verified*, not a finding. The workspace memory holds the full capture as
`research/harness-engineering-barbaste-2026` and `insights/harness-engineering-barbaste-2026`.

## 1. What the paper is

- **Definition it settles on:** an agent is a model plus a **harness** — the runtime that couples an LLM to
  the world through a loop, tools, context management, safety controls, orchestration and extension
  surfaces. The algebra `agent = model + harness` entered circulation in early 2026 (hashimoto in passing,
  Trivedy in LangChain's Deep Agents series) and this paper is the empirical follow-up.
- **Method:** source-code anatomy of eleven production coding harnesses — about **4 million lines** of
  Python, TypeScript and **Rust** — organised against seven canonical subsystems (§2). It describes and
  compares how the systems are built; it does not rank them.
- **Corpus:** Claude Code (Anthropic), Codex CLI (OpenAI), Gemini CLI (Google), Mistral Vibe (Mistral),
  OpenHands, Aider, Mini-SWE-Agent, **Hermes (Nous Research)**, **Pi**, and two more of the
  OpenCode/OpenClaw class — with Databricks' **Omnigent** as a **meta-harness** contrast point (it wraps
  Claude Code, Codex, Cursor, OpenCode, Hermes, Pi and YAML-defined agents behind one API with cross-harness
  policies, a uniform OS sandbox and shareable multi-device sessions).
- **Deliverables:** 13 cross-cutting observations, a catalogue of **29 recurring design patterns**, **18
  design recommendations** pinned to observed code, and a **90-line minimum-viable-harness scaffold**
  implementing ten of them.
- **Methodological bonus:** the original eight systems were re-pinned rather than replaced, so the corpus
  doubles as a **controlled longitudinal sample** — same harnesses, source-diffed across one quarter.

**Caveat recorded because it is easy to mis-cite:** secondary summaries disagree on the eleventh system
(`OpenCode` vs `OpenClaw`). Cite the arXiv page, not a thread, when the exact corpus matters.

## 2. The seven subsystems — the useful thing to steal

Every harness in the sample is described against the same seven parts. That makes it a free audit frame:

| subsystem | what it covers | pii's counterpart | status |
|---|---|---|---|
| the loop | the turn loop, retries, termination | `pii-core` + `pii-worker` | **to verify** |
| LLM-provider layer | clients, retries, streaming, routing | `pii-model` | **to verify** (025 planned routing/failover) |
| tools | tool registry, schemas, execution | `pii-tools` | **to verify** |
| memory / context | what enters the window, compaction | `pii-memory` | **to verify** |
| safety / permissions | what may run, approval, sandbox | gate + hooks | **to verify** |
| orchestration | sub-agents, scheduling, parallelism | `pii-worker`, `pii-graph` | **to verify** |
| extensibility | skills, MCP, plugins, hooks | `pii-mcp` + `AGENTS.md` + skills | **to verify** |

The value is not the table — it is that a third party already filled it in for eleven systems, so pii can be
graded against the same axes instead of against taste. **Filling this table honestly is the whole first task
(§6).**

## 3. The two absences, which are the paper's sharpest finding

After three rounds of corpus expansion, two things are still true across ~4M lines:

1. **No agent runtime imports a general-purpose agentic framework.** No LangChain, no LangGraph, no AutoGen
   — and Gemini CLI uses neither of Google's own. Harnesses are hand-rolled.
2. **None retrieves code with vector embeddings.** Retrieval is deterministic: ripgrep, tree-sitter, glob,
   and auto-discovered Markdown context files (`AGENTS.md`-class).

Both cut against the two most common architecture suggestions made to a project like pii ("adopt a framework",
"add a vector store for code search"). They are not arguments that frameworks or embeddings are bad — they
are evidence that the systems people actually run every day found them unnecessary, and that the deterministic
path is what survives contact with a real repository.

**Relevant to pii specifically and to be checked before claiming:** does `pii-memory` use embeddings at all?
If it does, that is a deliberate difference from all eleven systems and should be justified as one. If it does
not, this note is the citation for why pii's memory crate is a knowledge store rather than a vector index.

## 4. Skills lead MCP, and pii already carries both

Measured adoption in the sample: **`SKILL.md`-style skills 9/11, MCP 8/11**. The paper documents them as
complementary — skills as progressive context disclosure, MCP as the tool protocol — plus a third role for
ACP (6 systems) and the emergence of "harness hosting" (OpenHands runs Claude Code, Codex or Gemini CLI as
interchangeable backends).

pii ships `pii-mcp` and a skills/`AGENTS.md` surface, which is the shape the corpus converged on
independently. The check worth doing is not "do we have both" but whether pii's skills are *progressively
disclosed* (one-line snippets, schema on demand) the way the paper reports the leaders doing it, or loaded
whole.

## 5. Platformisation, imitation, and the meta-harness

The thesis of the second edition: in the first half of 2026 the coding harness went **from tool to
platform**. Harnesses became SDKs; marketplaces, switching-cost tooling and governance layers appeared; the
agent became addressable as a model behind an OpenAI-compatible endpoint; and a meta-harness (Omnigent)
started arbitrating eleven vendor harnesses behind one API.

The longitudinal diff shows convergence hardening into **imitation**: Codex adopted Claude Code's hook
vocabulary verbatim and ships an importer for its sessions and settings; OpenHands reads Claude Code's plugin
format. Behavioural policy migrates out of prompt prose and into **configuration**.

Two consequences for pii, one flattering and one not:

- **Flattering:** pii already treats behaviour as configuration (policy files, gate steps, plan rows) and
  pii-worker orchestration is the same territory as the meta-harness — the direction is confirmed.
- **Not:** "orchestrates several agents" is now table stakes, so it is not a differentiator. What the paper
  calls the expensive parts — arbitration, policies, sandboxing — is where the difference has to be made, and
  that is exactly where pii's deterministic gate and its ledger live.

## 6. What pii should do with this (ranked, all cheap)

1. **Fill §2 honestly** — one row per subsystem, `have / partial / missing`, each with the file or crate that
   proves it. Half a day, and it produces a real architecture review with an external reference behind every
   axis.
2. **Read the two sections that matter and have not been read:** the paper's **Pi** section and its **29
   patterns / 18 recommendations**. This note is abstract-level; anything pii says publicly about the paper's
   view of Pi must come from the body.
3. **Check `pii-memory` for embeddings** and decide explicitly (§3). Either way, write the reason down.
4. **Use the 90-line scaffold as a comparison, not a template.** If pii cannot do what a 90-line harness does
   — one loop, tools, context, a stop condition — the extra 90,000 lines owe an explanation.
5. **Cite the paper in `README.md`** as external validation of the design, with the honest qualifier that it
   is a preprint and does not rank.

## 7. Cross-references

- `079-dsh-superagent-evaluation-2026-11-09.md` — the governance side of the same question (authorisation,
  attention budget, dry-run for proactive work).
- `081-egm-evaluation-2026-11-09.md` — the review side (fresh-context reviewers, drift reports).
- `005-dsh-vs-pii-harness-analysis.md` — the earlier harness comparison this paper supersedes in scope.
- Workspace memory: `research/harness-engineering-barbaste-2026`,
  `insights/harness-engineering-barbaste-2026`.

## 8. Limits of this note

Abstract-level only; the per-system dissection, the pattern catalogue and the recommendations are unread, and
the two absences are quoted from the paper rather than re-measured here. The corpus list is quoted with the
eleventh-system ambiguity flagged. **Status: RESEARCH — no code changed, no spec written, nothing scheduled.**
