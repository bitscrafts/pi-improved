# pii research corpus — topic map (SSOT condensation, 2026-09-28)

**Why this file exists.** `_docs/06-research/` holds **42 documents, 954 KB**. Organised by *file*, five of them
look like five subjects; the *content* overlaps, and several are superseded. This map is the reviewable artifact
for condensing that corpus: **one subject → one canonical document**, everything else a pointer, evidence, or
archive.

**STATUS (2026-09-28): the merges in §3 steps 2-4 are DONE.** 10 documents were merged into 5
canonicals; the counts and the old→new mapping are in §5.

**Method:** `ssot-doc-organization` → `references/corpus-condensation.md`. Surveyed sizes and index coverage;
derived each doc's topic from its own `# ` heading and declared `Status:`; assigned each document exactly one
role. Nothing has been deleted or moved yet — this is the map, and it is itself the resume point.

---

## 1. The corpus in numbers

| fact | value |
|---|---|
| documents | **42** |
| total | **954 KB** |
| largest three | `002` pass 2 **137 KB** · `001` pass 1 **80 KB** · `003` pass 3 **52 KB** = **269 KB (28% of the corpus in one 3-pass set)** |
| orphan (nothing linked it) | **1** — `041-BRIEF-full-review.md` (fixed: now in INDEX + TOC) |
| `06-research/` section README | was **absent** for every section (fixed) |
| docs referenced in the workspace register | 36 / 42 |

## 2. Topic map — canonical vs evidence vs superseded

Order within a row is **authority**: the doc that *measured* and *supersedes* is canonical; the others are its
inputs. "Superseded" means a newer doc covers the same subject and **changes the conclusion**.

| # | subject | canonical | inputs / evidence | superseded (→ archive, with pointer) | action |
|---|---|---|---|---|---|
| T1 | **Architecture plan** (the whole system) | `002` pass 2 | `001` pass 1, `003` pass 3 (each *revises* the prior — pass 2 carries a table of 7 corrections to pass 1; pass 3 reframes pass 2 §9.5) | `001` (pass 1) once pass 2's corrections are folded | **fold** the surviving pass-1/pass-3 content into `002`, then archive `001`; keep `003` as the tier-model section of `002` |
| T2 | **Architecture conformance / full review** | `026` (2026-09 codebase analysis + phase 5) | `010` full adversarial review pass 1, `011` conformance review pass 1 | `010`, `011` — earlier passes with fewer crates read | **fold** anything still true into `026`; archive `010`/`011` with pointers |
| T3 | **The 2026-09-18 review + its ground truth** | `032` (findings, 12 ranked candidates → specs 62–69) | `031` ground truth (crate map, policy measured, deferred list) | — | **keep both** — this pair is the live base for the staged full review (`041`/`042`); `031` is evidence, `032` is findings |
| T4 | **Opus review entry point / decision package** | `030` (8 open decisions) | `029` coding-harness design questions | — | **keep**; it is the reviewer's front door and now points at the staged review |
| T5 | **Coding harness / DAG / orchestration** | `028` SDD orchestration checkpoint | `027` spec-38 verification, `005` dsh comparison, `013` dsh context-slice reference | — | **keep `028` as canonical**; `005`/`013` stay as inputs (they are the external-repo evidence) |
| T6 | **Graph engine & graph as a service** | `037` native graph service design | `006` graph engine architecture, `035` graphify research, `036` graphify integration | `035`/`036` if `037` settles the tooling-vs-native question | **fold** the decision into `037`; archive `035`/`036` behind pointers |
| T7 | **Model tiering & local inference** | `003` §tier model (inside the architecture plan) | `004` granite nano-tier probe, `014` local-model spec workhorse, `015` in-process llama-cpp, `017` tiny models, `023` llama.cpp on pi | `004`, `017` — probes; `023` is a how-to (→ `02-guides/`) | **fold** the durable numbers into `003`; move `023` to `02-guides/`; archive the probes |
| T8 | **Model routing / provider endpoints / failover** | `025` routing + failover plan | `019` generic provider endpoints | — | **keep both** — distinct scope (plan vs endpoint support) |
| T9 | **Memory subsystem** | `024` memory + MCP verified status | `008`/`012` evolve "boss" module, `009` its adversarial review | — | **keep `024`**; `008`/`009`/`012` are one subject — see T10 |
| T10 | **Self-improving "evolve" module (P4.7)** | `012` remaining plan (latest) | `008` deep design, `009` opus review pass 1 | `008`/`009` once `012` carries their conclusions | **fold**; archive `008`/`009` with pointers |
| T11 | **Feature candidates (unbuilt, research-stage)** | one per subject — see note | `016` ML apps, `019` providers (T8), `020` git-graph web UI, `021` MQTT, `022` Android app, `034` Needle 3, `038` Chromecast, `039` Jev, `040` Laya | none — these are **decision inputs, not duplicates** | **keep as a set**, but they belong in one *indexed cluster*, because they are all "researched, not scheduled" |
| T12 | **Docs/manuals & meta** | `018` manuals self-contained | `033` spec-65 guardrail violation | — | **keep**; `033` is an incident record → belongs with the register, not research |
| T13 | **This week's staged work** | `042` staged state | `041` the brief | — | **keep** — and these two are *operational*, not research; they should move once the review runs |

### Note on T11 (the "candidate" cluster)

These ten docs are **not duplicates of each other** — each answers a different question, and the skill's trap
list is explicit that different *subjects* must not be merged just because they share a shape ("researched, no
code yet"). Their defect is **discoverability**, not redundancy: they are scattered among the numbered files
with no single entry point. The fix is one **index** (a "researched, not scheduled" table) — *not* a merge.

## 3. Proposed actions, in dependency order

1. **Done already:** index the orphan (`041`), add `042` to TOC, add the four missing section READMEs.
2. **T1 — the big one (269 KB, 28% of the corpus).** Fold pass-1 and pass-3 content into `002`; archive `001`
   with a pointer and a note of *which* corrections superseded it. This is the single largest readability win.
3. **T10 then T2** — same shape, smaller: fold `008`/`009` into `012`, and `010`/`011` into `026`.
4. **T6, T7** — fold the settled decisions into their canonicals; move `023` to `02-guides/` (it is a how-to,
   not research — the subject rule, not a merge).
5. **T11** — write the one-page index of "researched, not scheduled" candidates, linking all ten.
6. **T13** — after the staged review runs, move `041`/`042` out of `06-research/` (they are operational).

**Not proposed:** merging T11's candidates, merging the T3 pair, or touching anything whose conclusion is still
live. Nothing is deleted — superseded documents move to `99-archive/` with a dated pointer at the old path.

## 4. Resume point

A condensation of this size is 2–3 sessions. **This map is the resume point**: each row names its canonical, its
inputs, and its action, so a later session can pick up any single topic without re-deriving the survey.

**Scope caveat:** the pii **code** work is separate and unaffected — this map concerns `_docs/06-research/` only.
The staged full review (`042`) is still the pending item that must run before the pii intervention.

---

## 5. MERGE RESULT (executed 2026-09-28)

**Documents merged away: 10. Canonical documents that absorbed them: 5. Deleted: 0.**
Every absorbed document is preserved **verbatim** as a `## [merged] …` section in its canonical, and its
original file is archived (not deleted) in `99-archive/06-research-merged-2026-09-28/`.

| canonical (survives) | absorbed (archived) | why one document |
|---|---|---|
| `002-pii-architecture-plan-pass2.md` | `001-pii-architecture-plan.md`, `003-pii-architecture-plan-pass3.md` | passes 1→2→3 revise one another; pass 2 already carried a corrections table for pass 1 |
| `012-p47-remaining-opus-plan.md` | `008-pii-evolve.md`, `009-pii-evolve-opus-review-pass1.md` | design + adversarial review + remaining plan for ONE module |
| `026-codebase-analysis-and-phase5-plan.md` | `010-pii-full-review-pass1.md`, `011-pii-architecture-conformance-review-pass1.md` | earlier review passes over fewer crates; findings superseded |
| `037-pii-native-graph-service-design.md` | `035-graphify-code-knowledge-graph-research.md`, `036-graphify-integration-design-discussion.md` | the research behind a design that is now decided |
| `014-local-model-spec-implementation.md` | `004-granite-nano-tier-probe.md`, `017-tiny-models-real-usage.md` | measurements for one question (local models as workhorse) |

| measure | before | after |
|---|---|---|
| documents in `06-research/` | 43 | **33** |
| explicit cross-references repointed | — | **31** |
| content lost | — | **0** (verified: every absorbed body is present in its canonical) |
| documents deleted | — | **0** |

**Still to do (deliberately not merged):**
- **T11** — the ten "researched, not scheduled" candidates (`016`, `020`, `021`, `022`, `034`, `038`, `039`, `040`,
  `019`, `025`): they are **different subjects**, not duplicates. Their fix is one index page, not a merge.
- **T7** — `02-guides/023-llamacpp-granite-on-pi-process.md` should MOVE to `02-guides/` (it is a how-to, not research) —
  a relocation, not a merge.
- **`05-operations/033-spec65-guardrail-violation-doc-format.md`** — an incident record; belongs with the register.
- **T13** — `041`/`042` move out once the staged review runs.
