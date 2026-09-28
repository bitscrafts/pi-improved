# Researched, not scheduled — the pii candidate index (2026-09-28)

**What this is.** Ten research documents, each answering a **different** question, each with **no spec and no
code**. They are not duplicates of one another and are deliberately **not merged** — merging different subjects
destroys meaning. Their only real defect was discoverability: scattered among the numbered files with no single
entry point. This is that entry point.

**How to use it.** Pick by subject. Each row points at the one document that owns that subject. When one of these
becomes work, it graduates out of this index into a spec and its doc is filed as input to that spec — it does not
get a second research doc.

| subject | document | its own declared status |
|---|---|---|
| Classical ML applications in Pii (regression, classification, clustering) | [`016-ml-applications.md`](016-ml-applications.md) | no status declared |
| Generic OpenAI + Anthropic endpoint support for pii-model | [`019-generic-provider-endpoints.md`](019-generic-provider-endpoints.md) | RESEARCH — no code changed. Recommends a concrete integration |
| Porting vscode-git-graph to Pii's Web Viewer — Feasibility Study | [`020-git-graph-web-ui.md`](020-git-graph-web-ui.md) | DRAFT — research only, no implementation authorized. |
| MQTT for Pii — broker, bridge, or nothing yet | [`021-mqtt-broker.md`](021-mqtt-broker.md) | RESEARCH / FEASIBILITY — no code, no spec, no crate exists yet. |
| Android app — background monitor/actor for Pii events | [`022-android-events-app.md`](022-android-events-app.md) | no status declared |
| 025 — Model routing + provider failover in Pii: plan | [`025-model-routing-and-failover-plan.md`](025-model-routing-and-failover-plan.md) | PLAN (design + spec breakdown). No implementation. |
| Cactus Needle 3 — research and relevance to pii (2026-09-19) | [`034-cactus-needle3-automation-research.md`](034-cactus-needle3-automation-research.md) | research findings. No code, no spec. Written for later analysis. |
| pii as a Chromecast / Google Home device manager — feasibility (2026-09-19 | [`038-pii-chromecast-device-manager-feasibility.md`](038-pii-chromecast-device-manager-feasibility.md) | analysis for opus review. **No code written.** This is the technical |
| 039 — Jev (TypeSafe AI) as a System-1 decision layer for Pii: analysis | [`039-jev-system-one-decision-layer-analysis.md`](039-jev-system-one-decision-layer-analysis.md) | ANALYSIS ONLY. No implementation, no graph/spec/`.rs` change. |
| 040 — Laya on llama.cpp, containers, and how a System-1 model drives pii's | [`040-laya-on-llamacpp-and-graph-driving.md`](040-laya-on-llamacpp-and-graph-driving.md) | RESEARCH + MEASUREMENT. Infrastructure change made (container rename, |

## What is NOT in this index, and why

- **Merged/subject-different docs** — see [`000-TOPIC-MAP.md`](000-TOPIC-MAP.md) for the corpus map and the
  merge result (10 documents absorbed into 5 canonicals).
- **Decided subjects** — where a design doc settled the question (graph service `037`, architecture `002`,
  local models `014`), the research is a `## [merged]` section of the canonical, not a candidate here.
- **`02-guides/023-llamacpp-granite-on-pi-process.md`** — not a candidate: it is a **how-to** and was moved to
  `_docs/02-guides/`.
- **`05-operations/033-spec65-guardrail-violation-doc-format.md`** — not research: it is an **incident record** and was moved
  to `_docs/05-operations/`.
- **`041`/`042`** — operational (brief + staged state for the pending full review), not research.
