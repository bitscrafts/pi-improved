# Pii Manuals — Making the PDFs Self-Contained + Documenting Design Patterns

**Date**: 2026-09-10 · **Author**: orchestrator (research) · **Status**: PLAN —
enumerates every fix needed in a follow-up implementation pass; no `.tex` file
has been touched by this document.

## 1. The requirement (restated precisely)

The two manuals — `_docs/manuals/user-manual.tex` (admin) and
`_docs/manuals/technical-manual.tex` (developer) — compile to PDFs. A reader
of the PDF has **no filesystem access to `_docs/`**. Every `\texttt{\_docs/...}`
reference is currently a dead pointer to a reader who only has the PDF. The
fix is per-reference: **inline** the essential fact, **drop** the reference
if the manual already states the fact, or **move** the fact to whichever of
the two manuals is its canonical home (SSOT is *between the two manuals*,
never the `_docs/` tree — enforced already via `xr`/`\externaldocument` in
both files, lines 57–58 of user-manual.tex and 51–52 of technical-manual.tex).
The two manuals already cross-reference each other correctly via
`Chapter~\ref{...}`; that pattern is correct and should be extended, not
replaced.

Both manuals' frontmatter (user-manual.tex:94–99, technical-manual.tex:90–95)
currently states the SSOT is "this manual + the `_docs/` research tree." That
statement itself must change: the SSOT is between the two manuals only.

## 2. Every `_docs/`-or-absolute-path reference, with disposition

### user-manual.tex

| Line(s) | Reference | Disposition |
|---|---|---|
| 98 | frontmatter: "...and in the project's `_docs/` research tree" | **Rewrite**: drop the `_docs/` mention; state SSOT is between the two manuals only (mirror technical-manual's corrected frontmatter, see §4). |
| 152–154 | "documented in the SSOT at `_docs/01-overview/01-overview/001-pii-vision.md`" | **Inline**: the vision content (4 pillars, phases table) is already summarized at user-manual.tex:137–150 (Overview chapter) almost verbatim from `01-overview/001-pii-vision.md` §"Four Pillars". Drop the `_docs/` pointer sentence entirely; keep only "The architecture is in the companion Technical Manual, Chapter~\ref{chap:arch}." Nothing of vision.md is *not* already present except the phases table and the 3 open decisions (both stale/historical — phases 0–5 are complete by now per the repo's spec history; do not inline stale planning tables into a manual documenting the *shipped* system). |
| 213–214 | "documented in the SSOT at `_docs/02-guides/feature-gating.md`" | **Move + inline**: feature-gating is architecture/design-rationale content → belongs in the Technical Manual (already partially there, technical-manual.tex:185–200). Replace this sentence with `See the Technical Manual, Chapter~\ref{chap:arch}, \S\ref{sec:feature-gating}.` (add a `\label` to that Technical Manual section — see §4 below). |
| 472–474 | graph engine "...and in the SSOT at `_docs/06-research/006-pii-graph-engine.md`" | **Drop the `_docs/` clause**; keep only "The graph engine is documented in the Technical Manual, Chapter~\ref{chap:graph}." The Technical Manual chapter already contains the engine design (spec model, node kinds, capabilities/effects, execution/journal) — see §3 for what's still missing there. |
| 550–551 | "The design is documented in the SSOT at `_docs/02-guides/pii-mcp-bridge.md`" | **Inline** into the User Manual's MCP chapter (chap:mcp): the "why MCP not WASM," the "MCP is the integration surface, not the control plane" nuance (already partly quoted at 560–565), and the crate-shape/quarantine-rmcp rationale. See §3.3 for exact content to add. |
| 588–589 | evolve "deep design is in the SSOT at `_docs/06-research/012-p47-remaining-opus-plan.md`" | **Drop** the pointer; the operationally relevant facts (launcher behavior, state.md, daemon owns detection/gating/ledger) are already stated in the paragraph immediately above (582–586). Anything deeper (model routing, dev/prod promotion mechanics) is architecture → belongs in the Technical Manual (§3.4), referenced via `Chapter~\ref{chap:evolve}` (already shared chapter label between both manuals — note this is a **label collision risk**, see §5). |
| 718 | "See `_docs/02-guides/feature-gating.md`" (Operational Notes) | **Replace** with `See the Technical Manual, \S\ref{sec:feature-gating}.` (same target as line 213–214). |
| 722–728 | "Where to go for more": Research SSOT `_docs/06-research/`, Guides `_docs/02-guides/`, Vision `_docs/01-overview/01-overview/001-pii-vision.md` | **Delete this itemized list entirely** and replace with a short paragraph pointing only at the Technical Manual (the one legitimate cross-file pointer) — see §4 for suggested replacement text. A PDF reader cannot open a guides/ or research/ directory; listing them is actively misleading. |

### technical-manual.tex

| Line(s) | Reference | Disposition |
|---|---|---|
| 94–95 | frontmatter: "Where a fact has a canonical home elsewhere (the `_docs/` research tree), this manual links to it" | **Rewrite**: same fix as user-manual.tex:98 — SSOT is between the two manuals only. |
| 135–136 | "full vision and the four pillars are in the SSOT at `_docs/01-overview/01-overview/001-pii-vision.md`" | **Drop the pointer clause.** The four pillars are already fully listed at technical-manual.tex:140–151 (this *is* the inlined content already — the sentence at 135 is now redundant with what follows and should just introduce it without the dead path). |
| 192 | "See `_docs/02-guides/feature-gating.md`" | **Inline the remaining content** here (this is feature-gating's true home). Add: the "gate surfaces with risk, never the engine substrate" rule, the full features table (evolve/web/tui/channels/tls/memory/sandbox/github/provider-\*), and the worked `tui` feature example (Cargo.toml + `#[cfg(feature = "tui")]`) from feature-gating.md §"Mechanical pattern". Add a `\label{sec:feature-gating}` on this section so the User Manual can point at it precisely (closes the two User Manual references above). |
| 298–299 | "full engine design is in the SSOT at `_docs/06-research/006-pii-graph-engine.md`" | **Drop the pointer**; cross-check against 006 (see §3.2) — one paragraph of *design rationale* (why lane-allocation/`git log --graph` inspired `pii graph show`, §8 of 006) is not yet in the manual and is worth a short "Design Note" box; everything else in 006 is already inlined in technical-manual.tex §§303–452. |
| 481–482 | "This is the same pattern DeepSeek Harness (dsh) uses...See `_docs/06-research/005-...md`" | **Drop the pointer**, keep the sentence (it stands on its own as a factual claim); the comparative depth belongs in the "Technical Analysis" chapter which already summarizes it (772–781). No new content needed — see §3.5. |
| 727–729 | evolve "deep design...SSOT at `012-p47-remaining-opus-plan.md`. The opus review and remaining-work plan are at `009-` and `012-`" | **Drop all three pointers.** These are internal planning-log documents (dated, written as "user's ask verbatim," decision logs, research citations) — not manual material. Inline only the *current, load-bearing facts* that are missing from the Evolve chapter: the model-routing table and the dev/prod promotion mechanism actually implemented (`xtask promote-build`, per commit `33a97ca`/`72d1aa2` — verify against `xtask/src/promote_build.rs` before writing prose; this doc does not assert its exact current CLI surface). See §3.4. |
| 781 | "See `_docs/06-research/005-dsh-vs-pii-harness-analysis.md`" (Technical Analysis §"Convergent architecture") | **Drop the pointer.** The comparison table's essence (session/history model, persistence seam, tool registry, capability attenuation — dsh vs pii, "same pattern independently converged") is worth **one inlined comparison table** since it directly supports the event-sourcing design-pattern writeup (§4.7 below). See §3.5 for the exact table to add. |
| 789–790 | "...targets the `compaction.rs:69` no-op. See `_docs/06-research/013-...md`" | **Drop entirely**, including the sentence itself, not just the path. This describes **unbuilt, deferred future work** (013's own header: "REFERENCE ONLY...do NOT implement from this doc yet"). A manual documenting the shipped system should not cite an internal file:line for a known bug in unshipped work. Replace the whole "Context management (Slice C)" section (783–791) with either (a) removal, or (b) a two-sentence forward-looking note with no path/line citations: *"Context-usage introspection (a projection over the run journal showing per-request token composition) is planned but not yet implemented."* Recommend (b) since the section header is otherwise orphaned. |
| 799 | "See `_docs/06-research/006-pii-graph-engine.md`" (graph engine vs git log section) | **Drop the pointer**; the section (792–800) is already fully self-contained (topological layout, no-lane-for-loops decision) — no new content needed. |
| 805–808 | "Where to go for more": Research SSOT, Guides, Vision | **Delete**, same reasoning as user-manual.tex:722–728. Replace with a short paragraph pointing only at the User Manual. |

## 3. Content to inline (details)

### 3.1 Vision / four pillars (user-manual.tex Overview, technical-manual.tex Architecture Overview)
Already adequately duplicated-on-purpose (each manual states the 4 pillars once, which is fine — SSOT is *between the two files*, and each needs the pillars for standalone readability; this is the one acceptable exception to strict no-duplication because both manuals need the vocabulary independently). No action beyond dropping the dead `_docs/` pointers.

### 3.2 Graph engine — design-rationale gap (technical-manual.tex chap:graph, technical-manual.tex chap:analysis §"graph engine vs. git log")
Section 792–800 already carries the *what* (lane allocation, why loops don't port). What's thin: the *why bother* — `pii graph show` exists so a human can read a graph's structure without running it; 006 §8.4 records it renders `graphs/starter/*.toml` deterministically, ASCII-only, validate-first. Consider one added sentence in 792–800: *"`pii graph show <file>` (Chapter~\ref{chap:run}) is the CLI surface for this rendering."* — links forward to the User Manual command, closing a currently-missing cross-reference (the User Manual's Graphs section, line 469–484, doesn't mention `pii graph show` in its command box at all — add it there too).

### 3.3 MCP bridge (user-manual.tex chap:mcp)
Inline from `pii-mcp-bridge.md`, condensed to 2 short paragraphs:
- Why MCP was chosen over WASM for this seam: MCP is the standard inter-process tool-exposure RPC that external clients auto-discover; WASM solves portable/isolated guest code execution, a different problem (adding a node kind here is one `reg.register()` call, not a sandboxing problem).
- Crate shape note: `pii-mcp` exists partly to quarantine the `rmcp` dependency (and its HTTP-stack requirement) away from the rest of Pii — this is a dependency-isolation instance of the same "feature-gate optional/risky deps" pattern documented in the Technical Manual's feature-gating section (cross-reference it).
Drop everything else from that doc (the "Wiring" `hermes mcp add` example names a specific external tool the manual's audience may not use — already adequately covered by the existing "server exposes tools, not the control API" warnbox at 560–565).

### 3.4 Self-Improving Boss — missing current-state facts (technical-manual.tex chap:evolve)
Before writing prose, **verify against code** (this research pass did not do a full read of `xtask/src/promote_build.rs`, `crates/pii-evolve/src/*.rs`, or the routing table wiring — only confirmed the files exist and grepped for symbol names). Add, once verified:
1. A short "Model routing" subsection: which routing steps exist (`localize`, `patch`, `review`, `research`, `orchestrator` — already stated in technical-manual.tex §"Model routing (spec 28)," chap:model) and how the boss's own graph nodes select a step via `route_step` (cross-reference chap:model instead of duplicating).
2. A "Dev/prod promotion" subsection describing the actual `xtask promote-build` mechanism and the runtime-cycle gate (per commit `33a97ca`, "Slice F dev/prod + gated promotion") — pull the real flag/command surface from the xtask source, not from the aspirational 008 doc (008's `~/.pii-dev/` + git-worktree description is a *design proposal*, not necessarily what shipped — must be checked before inlining as fact).
3. The guardrails list from 008 §6 is durable, non-code-dependent policy (never operate on prod, require reproduction before patching, budget-bound every attempt, rate-limit per failure signature, provenance trailers) — safe to inline verbatim as a bulleted list; these read as committed design invariants, not planning-log noise.

### 3.5 dsh comparison table (technical-manual.tex chap:analysis, supporting the event-sourcing pattern)
Inline a condensed 4-row comparison table (session/history model, persistence seam, tool registry vs. model schema, capability attenuation) from 005 §3.1 — it is short, factual, and directly substantiates the event-sourcing and capability-attenuation design patterns (§4 below), so it earns its place instead of being a bare citation.

## 4. Suggested frontmatter / closing-section rewrites

Both manuals' `\lowertitleback` SSOT paragraph (user-manual.tex:94–99, technical-manual.tex:90–95) should read (adapt per manual):

> **SSOT** — This manual and the companion manual are each other's single
> source of truth: operational facts (install, configure, run, debug) live
> in the User Manual; architecture and technical analysis live in the
> Technical Manual. Neither manual duplicates a fact the other already
> states — it links to it instead.

Both manuals' closing "Where to go for more" sections (user-manual.tex 720–728, technical-manual.tex 801–808) collapse to one paragraph each, e.g. (User Manual side):

> **Where to go for more.** This manual is operational; architecture,
> design patterns, and technical analysis are in the companion Technical
> Manual — start at Chapter~\ref{chap:arch}.

(Technical Manual side, symmetric, pointing at User Manual Chapter~\ref{chap:overview} or chap:run.)

## 5. A structural problem to fix while editing: duplicate `\label{chap:evolve}`

Both manuals currently define **their own** `\chapter{The Self-Improving Boss}` with the **same label** `\label{chap:evolve}` (user-manual.tex:569, technical-manual.tex:717). Under `xr`, a same-named label defined in *both* documents is not a cross-document conflict (each document's own `\ref` resolves locally first), but it means `\ref{chap:evolve}` from the User Manual could ambiguously suggest its own chapter number rather than the Technical Manual's — verify in the compiled PDF that any `\ref{chap:evolve}` used *from the other document* via the `xr`-provided external reference actually resolves to the intended target (`xr` typically requires a distinct external command, not bare `\ref`, precisely to avoid this — check whether the existing cross-references at technical-manual.tex:301, 410, 452, 465 actually render correctly, since this doc did not compile the PDF to confirm). This is a **pre-existing latent bug**, not something introduced by the reference cleanup — flag it to whoever implements this plan.

## 6. Design patterns to document (with module + file locations)

Add a new chapter to the **Technical Manual** (e.g. `\chapter{Design Patterns}` placed after Architecture Overview, before The Capability Context) cataloguing the following. Each entry below is verified against current source (file:line as of this research pass).

### 6.1 Strategy — `ModelProvider` trait
- **Where**: `crates/pii-model/src/lib.rs:118`, `pub trait ModelProvider: Send + Sync`.
- **Why**: swap LLM backends (Ollama today, others later) behind one sync interface without touching call sites. Deliberately **sync**, documented in the doc-comment at lines 1–6: `GraphNode::run` is sync and executes on a blocking thread, so the provider seam must be sync too — a design constraint propagated from the caller down, not an accident.

### 6.2 Strategy + Factory — `GraphNode` trait + `NodeRegistry`
- **Where**: trait at `crates/pii-graph/src/node.rs:10` (`pub trait GraphNode: Send + Sync`); factory/registry at `crates/pii-server/src/registry.rs:33-34` (`NodeRegistry::builtin()`, `reg.register(...)`).
- **Why**: every node kind (transform, route, agent, tool, gate, heal_detect, ...) is a `GraphNode` implementation looked up by kind string at graph-load time — new node kinds are added by registering an implementation, not by branching in the scheduler. 005 §3.2(a) identifies the current limitation: the registry is fixed at compile time (no dynamic/WASM-loaded node kinds yet) — worth one sentence noting this as a documented boundary, not a gap to silently omit.

### 6.3 Capability pattern / attenuating context — `pii-core::Ctx`
- **Where**: `crates/pii-core/src/ctx.rs:38` (`pub struct Ctx`), `:209` (`fn child(&self, principal, narrow, share)`).
- **Why**: a `Ctx` cannot be un-narrowed — `child()` only intersects (never unions) the parent's capability set, so no code path can accidentally acquire more authority than its parent handed it. This is the mechanism behind the three-layer `run caps ∩ graph caps ∩ node caps` intersection already documented in chap:graph/chap:ctx — this pattern section should cross-reference those chapters rather than restate the capability bits table.

### 6.4 Bit-flag intersection — `CapSet` / `Effects`
- **Where**: `crates/pii-abi/src/cap.rs:10` (`bitflags!` for capability bits), `crates/pii-abi/src/effects.rs:11` (`bitflags!` for effect classes); intersection logic at `crates/pii-core/src/cap.rs:73`/`:142`/`:209` (`fn intersect`).
- **Why**: capabilities and effects are both closed, stable-forever bit sets (not open trait objects) specifically so that intersection is a cheap, total, auditable operation — a deliberate trade of extensibility for verifiability at a security boundary.

### 6.5 Builder with deny-wins semantics — `with_deny`
- **Where**: `crates/pii-core/src/cap.rs:56` (`pub fn with_deny(mut self, deny: Vec<PathBuf>) -> Self`), used at `crates/pii-core/src/cap.rs:270` to protect `/wt/specs` from the self-healing boss.
- **Why**: this is not a plain builder (which would just add allow scope) — the deny list wins even against a broader allow, which is the mechanism that stops the self-improving boss from patching its own oracle (specs/tests). Cross-reference chap:evolve §"Protected paths."

### 6.6 Facade / single-writer — `pii-store::Store`
- **Where**: `crates/pii-store/src/lib.rs:5` (module doc: "A `redb::WriteTransaction` is opened and committed inside one method body"), `:34` (`pub struct Store`), `:131` (`fn begin_write_counted`).
- **Why**: every mutation goes through one facade type that owns transaction lifetime end-to-end (never stored in a struct, never returned across a call) — this is what makes "one write txn per journal event" a provable invariant rather than a convention someone can violate from outside the crate.

### 6.7 Event sourcing — the append-only run journal
- **Where**: `RunEvent` enum at `crates/pii-abi/src/journal.rs:64`; replay/resume in `crates/pii-graph/src/run.rs`.
- **Why**: message/state history is *derived* by replaying the journal, never stored redundantly — the same convergent pattern independently implemented by DeepSeek Harness's session log (see the comparison table added per §3.5). Document this pattern with the dsh table as corroborating evidence that it's an emergent standard, not a local idiosyncrasy.

### 6.8 Observer — journal-to-bus bridge
- **Where**: `trait Observer` at `crates/pii-core/src/observe.rs:40`; accessor `Ctx::observer()` at `crates/pii-core/src/ctx.rs:307`; consumer `struct Bus` at `crates/pii-server/src/bus.rs:19`.
- **Why**: node execution code calls one `Observer` method per event without knowing who's listening; the live TUI/`event.subscribe` subscribers and the durable journal are two independent observers of the same notification, decoupled from the scheduler that produces them.

### 6.9 Dependency inversion — `Arc<dyn ...>` seams
- **Where**: representative seams — `HttpTransport` trait at `crates/pii-core/src/net.rs:12` (consumed by `pii-channels`, keeping it testable with a fake and keeping `cargo xtask net-check` enforceable); also present in `pii-graph/src/node.rs`, `pii-model/src/lib.rs`, `pii-tools/src/lib.rs`, `pii-net/src/{grant,listen,service}.rs`.
- **Why**: every crate boundary that could otherwise force a concrete dependency (a real socket, a real HTTP client) instead depends on a trait object, so tests substitute a fake and the `xtask` dependency gates (`deps-check`, `net-check`) can mechanically verify no crate smuggles in a forbidden concrete dependency.

### 6.10 SSOT-by-cross-reference — `xr` between the two manuals
- **Where**: `\usepackage{xr}` + `\externaldocument{technical-manual}` (user-manual.tex:57–58) and the reverse (technical-manual.tex:51–52).
- **Why**: this is the documentation-layer instance of the same "single canonical owner, everyone else references it" principle applied everywhere else in the codebase (the store owns the DB, the journal owns history, `pii-abi` owns the capability bits). Worth stating explicitly as the meta-pattern tying the whole catalogue together — a nice closing note for the new Design Patterns chapter.

## 7. Summary of implementation-pass scope (for the follow-up spec)

1. Rewrite both `\lowertitleback` SSOT paragraphs (§4).
2. Fix/remove all 16 `_docs/`-path references enumerated in §2 (8 per manual) per the per-row disposition.
3. Add `\label{sec:feature-gating}` to the Technical Manual's feature-gating section; repoint the two User Manual references at it.
4. Inline: MCP why-not-WASM rationale (§3.3), evolve model-routing + dev/prod promotion facts **after verifying against `xtask/src/promote_build.rs` and `crates/pii-evolve/src`** (§3.4), dsh comparison table (§3.5).
5. Replace the Slice-C/`compaction.rs:69` sentence with a path-free forward-looking note, or delete it (§2, technical-manual.tex:789–790).
6. Add the new "Design Patterns" chapter to the Technical Manual (§6, 10 entries), each with file:line and a one-paragraph rationale; cross-reference from the User Manual wherever an operational chapter currently explains a mechanism without naming its pattern (e.g., chap:debug's capability-denied section could point at §6.3/§6.4).
7. Flag (do not silently fix) the duplicate-`chap:evolve`-label cross-document `xr` question in §5 — needs a PDF compile to verify, out of scope for a non-compiling research pass.
8. Rewrite both "Where to go for more" closing sections to point only at the companion manual (§4).

Every disposition above keeps the fact in exactly one of the two manuals; every remaining cross-manual mention is a `Chapter~\ref{...}`/`\S\ref{...}`, never a filesystem path.
