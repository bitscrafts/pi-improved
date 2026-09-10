# Classical ML applications in Pii (regression, classification, clustering)

**Date**: 2026-09-10 · **Author**: research agent (claude-sonnet-5) · **Status**: DRAFT — proposal, nothing implemented

## Context

Pii already produces three structured, labeled-by-construction data sources
that are large enough and clean enough to train classical (non-LLM) models
on: the event-sourced run **journal** (`RunEvent` in `pii-abi/src/journal.rs`),
the **heal ledger** (`HealAttempt` in `pii-store/src/heal.rs`), and the
**memory store** (BM25 + i8 vectors + graph lane, `pii-memory/`). All three
are populated by normal operation — no new instrumentation needed to get a
first dataset, only an export/label pass over what's already durable in redb.

**Hard constraints that shape every proposal below**:
- `#![forbid(unsafe_code)]` is a workspace lint (`Cargo.toml` `[workspace.lints.rust]`).
  This does not ban ML crates — it bans `unsafe` in *Pii's own* code. Pure-Rust
  crates whose public API needs no `unsafe` at the call site are fine; crates
  that force `unsafe` usage in the caller are not.
- No GPU (confirmed in `004-granite-nano-tier-probe.md`: "no-GPU box"). Every
  model here must train/infer on CPU, ideally single-digit milliseconds at
  inference since some candidates (routing, no-op detection) sit in the hot
  path of a running graph.
- `missing_docs = "deny"`, `clippy::unwrap_used/expect_used/panic = "deny"` —
  any new crate's wrapper code must return `Result<_, PiiError>`, matching
  every existing module in `pii-memory`/`pii-store`.
- Pii already has a hand-rolled embedder (`TrigramEmbedder`, FNV-1a trigram
  hashing + L2 norm, `pii-memory/src/embed.rs`) and a hand-rolled i8 quantiser
  with dot-product similarity (`pii-memory/src/quant.rs`). These establish the
  house style: small, pure, dependency-light numeric code with property-based
  tests (`roundtrip_error_under_one_percent`), not a big ML framework pulled
  in wholesale. New ML code should follow the same shape where the model is
  simple enough to hand-roll (linear/logistic regression, k-means, decision
  stumps) and only reach for `linfa`/`smartcore` when the model family
  actually needs it (e.g. k-NN with a proper ball tree, or gradient boosting).

**Crate survey** (pure Rust, no required `unsafe` at call sites, CPU-only):
- **`linfa`** (`linfa`, `linfa-linear`, `linfa-clustering`, `linfa-trees`) —
  scikit-learn-shaped, built on `ndarray`. Best fit when the model needs
  training-set batch fitting (k-means, linear/logistic regression, decision
  trees) and the crate's own dependency tree is acceptable to add. `ndarray`
  itself does contain internal `unsafe` (BLAS-style indexing) but that is
  *inside `ndarray`*, not code Pii writes — the workspace lint only forbids
  `unsafe` in Pii's own crates, and `pii-store`/`pii-memory` already accept
  third-party unsafe transitively (e.g. `redb`).
- **`smartcore`** — similar scope to linfa, slightly more estimators (k-NN,
  naive Bayes, random forest), less actively maintained. Prefer over linfa
  only if a specific estimator (e.g. random forest for failure-mode
  classification) is needed that linfa lacks.
- **Hand-rolled** — for anything expressible as closed-form linear
  regression (normal equations on a handful of features), online logistic
  regression (SGD, a few dozen lines), or k-means on <10 dimensions, hand-roll
  it in the relevant crate (`pii-store` or a new `pii-ml` crate) the same way
  `decay.rs`/`quant.rs` hand-roll their numerics. This avoids a new workspace
  dependency for models that are 30 lines of arithmetic, and keeps the
  `#![forbid(unsafe_code)]` + `missing_docs` + `pedantic` lint discipline
  trivial to satisfy (no macro-generated code to document).

Recommendation: start every application hand-rolled; promote to `linfa` only
if a specific estimator (k-means with k-means++ init, decision trees) turns
out to be worth its dependency weight over a hand-rolled version. This is
consistent with `pii-memory`'s existing choice not to depend on an embedding
library for `TrigramEmbedder`.

---

## Regression

### R1. Predict run wall-time and token cost from graph shape + node history
**Data**: `RunEvent::NodeStarted{at_ms}` / `NodeFinished{dur_ms}` per node
across historical runs (journal, per-run, replayable via `run.rs::resume`'s
replay path); graph static shape from `GraphSpec` (node count, edge count,
presence of loops via `is_in_cycle`, node kinds). Aggregate per `(graph_name,
node_id)` a running mean/variance of `dur_ms` and token deltas — the atomic
`Budget::charge` calls already record exact token/tool-call/wall-ms debits
per node, so the label (actual cost) is exact, not estimated.
**Target**: predicted `max_wall_ms`/`max_tokens` needed for a *new* run of a
known graph, before executing it — used to size the `Limits` in `GraphSpec`
or warn the operator the declared budget is implausible.
**Features**: node-kind histogram, edge count, max fan-out, presence/count of
loop-budgeted nodes, and (once available) a small embedding of the agent
prompt via `TrigramEmbedder` (already exists, zero-cost to reuse).
**Model**: linear regression (ridge, to handle the collinearity between node
count and edge count) is very likely sufficient — this is a small-feature,
tabular, non-time-series regression. Hand-roll via normal equations (a
`nalgebra`-free closed form over ≤20 features is a `Vec<f32>` matrix
inversion small enough to hand-roll, or use `linfa-linear`'s `FittedLinearRegression`
if the dependency is acceptable).
**Online/offline**: offline batch retrain (nightly or on N new completed
runs) — training data is only available after a run finishes, and budget
planning happens before a run starts, so this is inherently a train-offline,
predict-online split. No online/incremental fitting needed.
**Where it plugs in**: a new `pii graph estimate <file>` CLI subcommand, or
a pre-flight check in `run.rs::start` that warns (not blocks — non-authoritative)
when `Limits` looks too tight/loose relative to the model's prediction.

### R2. Predict heal-attempt success probability from patch features
**Data**: `HealAttempt` rows (`pii-store/src/heal.rs`) — `gate`,
`gate_before`/`gate_after`, `patch_hash`, `model`, `tokens_in`/`tokens_out`,
`attempt_count`, and the terminal `state` (Fixed/Escalated/Flake) as the
label. This is a genuinely regression-shaped problem if the target is a
continuous "expected tokens to fix" rather than a binary outcome (see C-series
below for the classification framing of the same data).
**Target**: expected `tokens_in + tokens_out` to reach `Fixed` state, given
`gate` name, `model`, and `attempt_count` so far — used by the heal loop
to decide whether to keep retrying vs escalate to a human, i.e. to set a
per-signature token budget instead of a fixed global one.
**Features**: one-hot `gate` name, one-hot `model`, `attempt_count`,
`tokens_in`/`tokens_out` so far, and whether `gate_before == gate_after`
(a proxy for "the patch didn't move the needle" from the prior attempt).
**Model**: linear regression or a small decision tree (`linfa-trees`) — the
gate/model categorical interactions likely aren't linear (some gates are
inherently harder), so a shallow tree may out-perform linear regression
without much complexity cost. Start linear; only add `linfa-trees` if
residuals show the categorical interaction actually matters.
**Online/offline**: offline, retrained whenever the heal ledger grows by
some threshold (it's small — `ATTEMPT_RING_CAP = 20` per signature, but
`list_attempts()` returns all signatures, so the retrain set is bounded).

---

## Classification

### C1. Classify a run's terminal failure mode from the journal
**Data**: the journal's terminal `RunEvent::Finished{terminal}` /
`NodeFailed{error, retryable, attempt}` events. `PiiError`'s variants
(`pii-abi/src/error.rs`) already give a clean, small label space:
`Denied` (capability), `BudgetExhausted` (budget breach), `Timeout`,
`Internal`/`Unavailable` (model/provider error), `Invalid`. **This label
already exists as a Rust enum** — the "classification" work is mostly
building the feature extractor (what *led up to* the failure), since the
label itself is exact and free (it's the error variant string already
stored in the journal / propagated to `Terminal::Failure{error}`).
**Where ML adds value over a plain `match` on the error string**: the raw
`error: String` in `NodeFailed`/`Terminal::Failure` is free text (e.g.
`Internal(format!("heal: open {}: {e}", ...))`), not always the clean enum
discriminant — errors get wrapped/formatted as they cross module boundaries
(see `heal.rs`'s `PiiError::Internal(format!(...))` everywhere). A classifier
over token features of the message (bag-of-words / the existing
`TrigramEmbedder`) generalizes to *new* wrapped-error phrasings a hardcoded
`match`/`contains` heuristic would miss — the same "silent no-op" problem
class spec 24 was built to catch (a declared-but-brittle heuristic silently
stops matching as error text evolves).
**Model**: multinomial logistic regression over trigram-embedded error text
(256-dim `TrigramEmbedder` output, already built, zero new embedding code)
plus small structured features (which node kind failed, retryable flag,
attempt number). This is squarely "hand-roll a 256->5 linear classifier
with softmax + cross-entropy SGD," ~50 lines, no new dependency — or
`linfa-logistic` if preferred.
**Online/offline**: offline batch training (label set is bounded by distinct
`PiiError` variants — 8 total); inference is online, run once per failed
run, cheap enough (single matrix-vector multiply on a 256-dim embedding) to
run synchronously when a run finishes.

### C2. Learned router: pick tier-0 (B1 embedder) vs tier-1 (small LLM) vs cloud
**Data**: `004-granite-nano-tier-probe.md` already defines the tier
boundaries (tier 0 = nano/no-reasoning/$0, tier 1 = local ~9B via
llama.cpp, presumably tier 2 = cloud). Training data: historical
`(prompt, tier_used, outcome)` triples where outcome is success/failure or
"had to escalate to the next tier" (an escalation is itself a hard negative
label for the tier that under-served the prompt). This data does not exist
yet as a durable table — it would need to be logged wherever tier selection
currently happens (today apparently a fixed default, "Tier 0 stays B1 by
default" per the probe doc) alongside the downstream outcome.
**Target**: which tier a new prompt needs, given only the tier-0 embedding
(no LLM call spent on the classification itself — the whole point of a
tier-0 router per pass-3 §3.1 is "one call, no reasoning, escalates").
**Features**: the `TrigramEmbedder`'s 256-dim vector of the prompt (already
computed for memory search — free reuse) plus prompt length, presence of
code fences, presence of a tool-call request.
**Model**: multinomial logistic regression, same shape as C1, over the
`Embedder` output — this is literally the "B1 = `Arc<dyn Embedder>` + fitted
head" architecture the tier-0 probe doc already specifies (`pass-3 §3.1`,
"Default backend is B1 (an `Arc<dyn Embedder>` + fitted head, pure Rust, no
chat model)"). **This is not a new idea — it's the missing "fitted head"
half of an already-decided architecture.** The embedder exists
(`TrigramEmbedder`); the fitted head (the classifier) does not yet.
**Online/offline**: train offline on logged (prompt, tier, outcome) triples;
infer online, in the hot path, before every agent/tool dispatch — must be
sub-millisecond, which a 256->3 linear layer easily is.

### C3. Detect "silent no-op" runs with a classifier instead of a heuristic
**Data**: spec 24 (`specs/24-silent-noop-check.md`) already builds a
**deterministic, no-model** source-text scanner for a *static* silent-no-op
class (declared RPC methods/events/config keys with no consumer). That
scanner is explicitly the right tool for its job (N1: "NO-MODEL — reads
source text ... A non-zero exit means a real miss, not a quota/blip") and
should not be replaced. The ML opportunity is a **different, dynamic**
no-op class the static scanner cannot see: a run that *executes* every node,
returns `Succeeded`, but produces no observable side effect worth the tokens
spent — e.g. a `tool` node whose `stdout` is empty/unchanged, or an `agent`
node whose output hash equals its input hash. **Recommendation: do not build
this with ML.** The journal already carries `value_hash` per
`NodeFinished` — a same-hash-in-equals-hash-out check, or an empty-stdout
check on `tool` nodes, is a deterministic predicate exactly like spec 24's
own scanner, not a fuzzy pattern a classifier is needed for. Listed here
because the brief asked for it explicitly, but the finding is negative: this
is a heuristic-shaped problem, not an ML-shaped one, and building a
classifier for it would be over-engineering relative to a `value_hash`
equality check plus an empty-output check — save the ML budget for C1/C2
where the label space is genuinely fuzzy (free-text error messages,
predicting a tier before an LLM runs).

---

## Clustering

### K1. Cluster past runs by behavior to find recurring failure patterns
**Data**: per-run feature vector built from the journal — sequence of
`(node_kind, port_taken)` pairs (e.g. `agent→err`, `tool→ok`, `route→fail`),
total dur_ms, total tokens, terminal `PiiError` variant (from C1). This is
the same underlying journal data as C1/R1, aggregated differently: C1 predicts
one run's label, K1 groups many runs to find labels nobody defined yet.
**Target**: no target — unsupervised. The value is surfacing clusters an
operator hasn't named: e.g. "all runs where `heal-fix`'s `route_retest`
fires twice then escalates" forming a tight cluster reveals a recurring
gate/model combination that never actually converges, which is exactly the
kind of finding `009-pii-evolve-opus-review-pass1.md` and the heal ledger
are trying to surface manually today (`HealState::Escalated` rows are
already collected one at a time; clustering would show *which escalations
look alike*).
**Features**: a fixed-length vector — for a small graph vocabulary (the
starter graphs have ≤10 node kinds), a bag-of-(node,port) count vector is a
clean, fixed-dimension input; no need for sequence models.
**Model**: k-means, hand-rolled (Lloyd's algorithm over a low-dimensional
count vector is ~40 lines) or `linfa-clustering::KMeans` if k-means++
initialization and silhouette-based k selection are wanted for free. k is
small (expect single-digit clusters for a repo this size); k-means over
hierarchical clustering is the pragmatic choice since cluster count doesn't
need to be dynamically discovered from a dendrogram — an operator picking
k=5-ish and inspecting is fine for a first pass.
**Online/offline**: fully offline, ad hoc — this is an analyst tool
(`pii graph cluster-runs` or a report script), not something that runs in
the hot path.

### K2. Cluster memory facts by topic for consolidation
**Data**: `pii-memory`'s existing i8-quantised vectors (`quant.rs`) over
`Doc` entries, already indexed for the vector search lane
(`vector_search`, `search.rs`). **This is the strongest "reuse, don't
build" case in the whole brief**: the feature vectors this clustering needs
already exist in the store for an unrelated purpose (nearest-neighbor
search), computed via the same `TrigramEmbedder` used elsewhere. No new
embedding code, no new feature engineering — only a clustering pass over
vectors already at rest in `MemStore`.
**Target**: no target — unsupervised. Used by `pii-memory/src/consolidate.rs`
(the existing `consolidate` maintenance op, exposed as the `memory` node
kind's `consolidate` op per the graph-engine doc) to group near-duplicate or
topically-adjacent atoms/scenes before summarizing or pruning them — turning
consolidation from "decay + drop old" (current `decay.rs` mechanism) into
"decay + merge similar."
**Model**: k-means or a simple density-based approach (DBSCAN via
`linfa-clustering`) over the existing i8 vectors — note `quant.rs`'s
`cosine_q` already gives a fast scaled-dot-product similarity, so a
k-means variant using that same similarity (rather than pulling in a new
distance metric) keeps consistency with the existing quantised-vector
infrastructure and avoids de-quantising back to f32 for clustering.
**Online/offline**: offline, run as part of the existing `consolidate` op
cadence (already a scheduled/triggered maintenance operation, not new
infrastructure to wire in — only a new step inside it).

---

## Prioritized recommendation: top 3 to build first

1. **C2 — Learned tier-0 router (fitted head for B1 embedder).**
   **Why first**: this is not a new feature, it is the missing half of an
   *already-decided* architecture (`004-granite-nano-tier-probe.md`,
   pass-3 §3.1: "B1 (an `Arc<dyn Embedder>` + fitted head)"). The embedder
   (`TrigramEmbedder`) and the trait boundary (`Embedder`) already exist and
   are load-bearing in `pii-memory`. Building this closes a known gap
   instead of opening a new research question.
   **Data source**: log `(prompt, tier_selected, escalated: bool)` at the
   dispatch site once tier selection exists as code (today it's a fixed
   default per the probe doc); bootstrap the very first model on a small
   hand-labeled set if no logged data exists yet.
   **Crate**: hand-rolled multinomial logistic regression over the existing
   256-dim `TrigramEmbedder` output — no new dependency.

2. **C1 — Failure-mode classifier over journal error text.**
   **Why**: the label space is free (8 `PiiError` variants, already an
   exhaustive Rust enum) and the training data already exists in every
   historical journal with zero new instrumentation. It directly extends
   spec 24's philosophy ("declared ⇒ used" made machine-checkable) into
   free-text error provenance, which a source-text scanner structurally
   cannot classify (wrapped/formatted error strings vary; the enum
   discriminant doesn't always survive to the surface).
   **Data source**: `Terminal::Failure{error}` / `NodeFailed{error}` across
   the run journal (redb, `pii-store`), replay via the existing
   `run.rs::resume` read path — no new store table.
   **Crate**: hand-rolled softmax regression over `TrigramEmbedder`
   features (reuse of C2's model shape) — no new dependency.

3. **K2 — Cluster memory facts for consolidation.**
   **Why**: highest reuse-to-new-code ratio of everything surveyed — the
   feature vectors (i8-quantised trigram embeddings) already sit in
   `MemStore` for the vector search lane, and the consumer
   (`pii-memory/src/consolidate.rs`) already exists as a scheduled
   maintenance operation. This turns an existing "decay-only" consolidation
   pass into a materially better one (merge, not just drop) for near-zero
   new infrastructure.
   **Data source**: `MemStore`'s existing quantised vectors, read via the
   same path `vector_search` already uses.
   **Crate**: hand-rolled k-means over `quant.rs`'s `cosine_q` similarity
   (keeps the existing quantised representation, no de-quantisation step);
   `linfa-clustering` only if DBSCAN's variable cluster count proves
   necessary in practice.

**Deliberately not recommended first**: R1/R2 (budget/heal-cost regression)
and K1 (run clustering) are real but lower-leverage — they need a longer
history of completed runs to train on before they'd out-perform the fixed
`Limits` operators already set by hand, whereas C1/C2/K2 have usable data
*today*. C3 (silent-no-op-via-ML) is actively **not recommended** — the
`value_hash`/empty-output check that would replace it is a one-line
deterministic predicate, not an ML problem; building a classifier there
would be solving a heuristic-shaped problem with the wrong tool.
