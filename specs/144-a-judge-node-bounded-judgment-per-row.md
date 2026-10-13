# Spec: a judge node — bounded judgment per row, thresholds in the graph

## Overview

A proactive agent needs one thing pii does not have yet: a cheap, bounded way to look at a stream of events
and say *normal / suspicious / incident* — one call per row — with the **threshold that opens real work living
in the graph, not in the model**. That is the layering Pi Durable's demo is built on
(`_docs/06-research/060-durable-agents-pi-durable-2026-10-13.md`): **code owns thresholds and side effects, a
small model owns bounded judgments, the agent owns open-ended work.** Measured there: a classifier call a few
hundred milliseconds, ~$0.00003 per row, an incident opening after 3 verdicts sharing one fingerprint within
120 s, an incident agent costing ~$0.09 — and the agent then waits for an approval.

pii already has every piece except the node: cron graphs (128) trigger work, `llamacpp-server:` (129) reaches a
local llama.cpp server, the store counts, and the engine opens runs. The judging layer therefore costs
**nothing per row** here, and it runs on a machine the user owns.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (proactive capability; 060 §2)
**Source of the finding**: `_docs/06-research/060-durable-agents-pi-durable-2026-10-13.md`; specs 128 (cron
graphs), 129 (`llamacpp-server:`), 131 (a refused run is a decision)

## Requirements

### Functional
- **R1** A `judge` node kind: given one row (text + optional metadata) and a declared set of labels, it makes
  **one bounded decision call** and returns a **typed** verdict — the label, a **calibrated confidence**, and an
  optional short reason. The call goes through the existing model resolution, so `llamacpp-server:<model>` (the
  local llama.cpp server) is the default and no new dependency is added.
  This shape is not invented here: it is the decision-layer pattern already researched in this workspace — a
  model that *picks* rather than writes, one call per row, with a confidence, at the seams of the harness
  (`research/jev-agent-harness-sam-witteveen` names the five seams: routing, gating, selection, ranking,
  triage; `research/jev-seven-usecases-ailabs` measures it). What is new is that it can be **local and open**:
  `TheoLeeCJ/SemIf-OpenJev` (4,810 stars, verified 2026-10-13, "semantic ifs from open models, on a 3090 at
  home") and a model fine-tuned for **typed decisions** such as `caiovicentino1/Eikos-27B` (781 downloads,
  pipeline `text-classification`, tags `decision-making`, `calibration`, `typed-decisions`, verified
  2026-10-13) are the open equivalents of the paid classifier the demo uses. Preferring them keeps the judging
  layer's marginal cost at ~0.
- **R2** A verdict is recorded where the graph can act on it: an event per verdict and a counter keyed by a
  **fingerprint** the graph supplies (e.g. the error class or the service), so a threshold can be expressed as
  a query over counters inside a window. The node never opens a run itself.
- **R3** A threshold is a **graph** decision, not a model decision: a graph (or a small `threshold`/`watch`
  node) opens an agent run when N verdicts with one fingerprint land within a window, and opens it **exactly
  once** for that window — the same idempotency discipline as channel posts (128), so a re-run does not open a
  second incident.
- **R4** A model error is a **recorded verdict of its own** (an error label with the reason), never a silent
  `normal`: a judge that cannot answer must not look like a row that is fine (the false-green rule,
  DO-NOT-REPEAT 1.50).
- **R5** The per-call budget is bounded: a stated timeout and a stated maximum output, both visible in the
  config or the node's own defaults, and a row that exceeds them yields the error verdict of R4.
- **R6** It runs unattended: a cron graph can drive the judge (128), so a stream of rows is judged without an
  operator, and the counter rate is visible in `pii metrics`.

### Non-Functional
- No new dependency; the OpenAI-compatible path (129) carries it. clippy `-D warnings` clean.
- The node does not read or write anything the graph did not grant: no capability is added by this spec.
- One row costs one call — no batching that would make the cost per row invisible.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `a_judge_labels_a_row_and_records_a_verdict` (new) | a scripted model returning a verdict | the verdict event + the counter for the supplied fingerprint |
| `a_model_error_is_an_error_verdict_not_a_normal_one` (new) | a model that fails | an error verdict with the reason; **no** `normal` |
| `the_threshold_opens_exactly_one_run_per_window` (new) | N rows with one fingerprint inside the window, the graph evaluated twice | exactly one run opened; the second evaluation opens none |
| `below_the_threshold_nothing_opens` (new, control) | N-1 rows | no run opened |
| `the_judge_runs_from_a_cron_graph` (new) | the cron-driven graph | verdicts accumulate with no operator present |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo test -p pii-graph 2>&1 | tail -2 | grep -q 'test result: ok'`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -1 | grep -qv 'error'`
- [ ] an example graph exists and is valid: `cargo run -q -p xtask -- deps-check 2>&1 | tail -1 | grep -qv 'FAIL'` and the graph file is named in the closure

## Guardrails

- Do NOT let the judge decide anything with a side effect: it labels, the graph acts. A model that can open a
  run directly is a model with a capability, and this project's whole safety story is that models do not hold
  capabilities.
- Do NOT default to a cloud model. The point is a judging layer that costs nothing per row; the local
  llama.cpp server is the default, and a cloud model may only be an explicit, documented override. Where a
  typed-decision model (a local semantic-if / distilled classifier) is available, it is the better judge than a
  chat model asked to label: the verdict is a decision, not prose.
- Do NOT let the judge's output be free text a caller must parse. The verdict is a typed value with a
  confidence; a caller compares labels and thresholds, never strings.
- Do NOT build a threshold inside the model's prompt ("open an incident if …") — thresholds are code/graph
  (the demo's own table says so).
- A test that passes with a judge that always answers `normal` is the defect this spec exists to prevent: make
  the error path and the below/at-threshold pair explicit.
- Do NOT create, edit or rewrite anything under `specs/`; do NOT commit; scratch under `/projects/_tmp/`.

## Closure

(outcome, the node's name and its schema, the counter names, the exactly-once proof, and the example graph —
filled in when the item is closed.)
