# Google Elephant-Goldfish Model (EGM): what it is, and what pii should take from it

Filed 2026-11-09. Evaluation only. Primary source: `github.com/google/elephant-goldfish-model`, read on 2026-11-09 (API metadata, the file tree,
`docs/designs/review.md`, `synthesizer.md`, `drift_report.md`, `architecture.md`, `agents/implementation_readiness.md`,
`resources/document_standards/implementation-readiness-standard.md`, `skills/egm-review-loop/SKILL.md`). amem holds the 2026-11-04 note
(`research/elephant-goldfish-model-egm`, `insights/elephant-goldfish-model-egm`); this note adds what the code and design docs show and maps it onto pii.

## 1. What it is (verified)

- Apache-2.0, Python, 301 files, created 2026-09-13. Stars: 11,762 on 11-04, **16,091 on 11-09** (the repo is moving fast: commits every few minutes today).
  83 open issues. Not an officially supported Google product (its own disclaimer).
- It ships **no runtime**. It is markdown: 7 skills (`egm-brainstorm`, `-draft`, `-review`, `-review-loop`, `-resolve`, `-bootstrap`, `-drift-report`),
  10 reviewer personas (`agents/*.md`), 5 document standards (`resources/document_standards/*.md`), and packaging for Claude Code, Codex, Cursor,
  Copilot, OpenCode and Gemini. Most of the 301 files are packaging and tests of the packaging, not the method.
- The method fits in one page: an **elephant** (the agent holding the project context) drafts; **goldfish** (read-only subagents with no
  conversation history, only the document and the files it cites) review it against a written standard; the elephant resolves findings; a human
  approves. Two properties are mechanical and matter: a goldfish has no write tools, and if a harness cannot make a fresh context the review
  STOPS rather than running in-context.

## 2. The three pieces that are worth reading closely

**(a) The review loop has a stopping rule that can fail** (`egm-review-loop`). Each round, fresh reviewers return findings tagged Blocker, Major or Nit.
A round is clean when no Blocker or Major is raised. A run is converged only after **two consecutive clean rounds**, because "the next fresh reviewer can
find a different gap in the same text". Cap is 5 rounds, and a finding raised again after it was settled counts as churn that stops the loop. A reviewer
that did not run makes the round NOT clean (a missing review is never a pass). The review set is decided from the results before the round's fixes are applied.
This is the check discipline pii already demands (a check must be able to fail, bounded attempts), written as an algorithm.

**(b) The Implementation Readiness Standard is a spec acceptance rule** (`implementation-readiness-standard.md`, 1.6 KB). Pass means a competent
engineer who does not know the codebase can implement the plan correctly on the first attempt "without guessing, asking, or making independent design
choices". It names a **Zero-Decision Rule**: any design choice left to the implementer fails the plan. It requires an explicit list of repo-relative
file paths to create, modify or delete, "suitable for enforcing a write allowlist", with no opportunistic adjacent refactors; steps precise enough
("add field X to struct Y in file Z"); phases that each leave the tree buildable; and a test plan with named cases.
This is almost exactly pii's `## Files` rule and the lane's `unlisted-changes` refusal, stated as the bar a reviewer applies BEFORE the worker is paid.

**(c) The drift report** (`egm-drift-report`). For each checked-in design doc, a fresh agent reads the doc, extracts the symbols, files and APIs it
names, compares them to the code, and reports ONLY actionable divergence, in four classes: missing feature, incomplete model, logic shortcut
(hardcoded where dynamic was designed), architectural divergence. Output: `YYYY-MM-DD_DRIFT_REPORT.md`, "No drift detected." when clean. It audits
documents one at a time so context does not degrade. It is recommended on a cadence, and not enforced.

Also worth noting: the synthesizer persona runs each reviewer **twice** (A/B) for recall, deduplicates, triages and writes one report, so the root
context never sees the raw swarm output.

## 3. Where pii already has it, measured

- pii's spec lane already is elephant + goldfish at the back of the pipe: a worker implements with no conversation history (a fresh brief), and
  `red-audit.sh`, the criteria runner, the `unlisted-changes` refusal and the gate judge it. The reviewer never sees the worker's reasoning, only the tree.
- pii's judge node (`kind = "judge"`) is a read-only-by-capability reviewer with an allowed-env list of four names. `method-judge.toml` exists.
- `doc-drift-check` is already a gate step (see the 22-step list): it checks documents against code, but mechanically.
- The attempt cap, the per-row session and the hold already bound loops. 

## 4. Where pii does NOT have it, and the evidence it matters

1. **No review of the SPEC before the worker is paid.** Measured this session: row 399 was refused 252 times over 12.4 hours before any worker ran; its
   spec premise was wrong (a control expected `cargo --version` to print from a scrubbed child); its file list missed producers and the gate caught
   them 25 minutes later. Row 401: the spec said "do not grow `discord.rs`" and the worker grew it 22 lines; the gate failed after 25 minutes.
   Every one of these is something a zero-context reader holding the Implementation Readiness Standard finds in minutes for a few cents:
   "which counter will judge the file size?", "list every producer of `tool = "bash"`", "this control assumes a toolchain the child does not have".
2. **No stopping rule of the two-clean-rounds kind.** pii's loops stop on attempts, not on a reviewer-converged signal.
3. **No drift pass of design notes against code.** The 17 notes under `_docs/06-research` and the specs are never re-read against HEAD; spec 374's
   criterion is false today because row 393 changed `doc-code`, and nobody noticed for days (open item 11).

## 5. What to take, ranked by value over cost, each as pii graphs on the existing 14 kinds

1. **`spec-review` graph** (highest value). Before a spec is queued: N `judge` nodes, each given ONLY the spec file and the files it names (a goldfish by
   construction: the node's context is its input, and its caps are read-only). Reviewer set: readiness (zero-decision, file list, test plan), a critic
   (what breaks, failure modes), and a **gate-parity reviewer** (new in pii: which gate steps will judge each file the spec touches, and does the spec
   say so — it would have caught rows 399 and 401). Output: findings as data with severity, one `route`: any Blocker or Major refuses the queueing with the
   finding text; two consecutive clean rounds queue it; cap 3 rounds, then one question to the human. A reviewer that errors counts as not clean.
   This composes existing nodes: `judge`, `route`, `refuse`, `call`, no new kind. It also feeds the multi-pii deliberation design (note 080): the
   proposer/critic/arbiter roles ARE this loop with the PR as the medium.
2. **Gate-parity facts handed to the worker** (from the size complaint): the worker's brief lists the size ratchet and each touched file's headroom,
   which is the pii-specific piece EGM does not have.
3. **`drift-report` graph, on a timer.** A `timer`, then per document a `judge` with read-only caps comparing it to the code it names, one `post` a week
   to the pii channel only when there is drift ("No drift detected." posts nothing: attention is the budget). Start with the specs and `_docs` notes that
   name files, and the 4 divergence classes. Cheap: one document per run.
4. **Severity vocabulary** Blocker / Major / Nit adopted as the verdict token of the judge output (it parses, and unparseable fails closed as in pidag spec-37).
5. **A/B sampling** for the two reviewers that matter: run each twice at different model seeds or two models and take the union of findings. Measure first
   whether the local model gives different findings on the second run; if not, it costs and returns nothing.

## 6. What not to take

- The packaging: six harness adapters, a docs site, an Astro site, release scripts. pii is one binary; none of it applies.
- The interactive turn-taking (the loop pauses for the user between rounds). pii runs unattended; the human decision point is discordance, per note 080.
- Personas as prompt files as the unit of reuse. In pii the unit is a graph with a role literal, which is testable and versioned.
- The claim that review replaces testing. EGM says so itself: "a review does not replace tests or human judgment". Every pii criterion stays a command.
- Dependence on the repo: a fast-moving, 11-day-old, unofficial project. Read it for the method; copy nothing.

## 7. Cautions

- A model reviewer is another model: it can agree with a wrong spec. The control is the same as for every other check in pii: it needs a case it must
  FAIL. Concretely, build a corpus from this session's real defects (the row 399 spec with its `cargo --version` control; the row 401 spec without a size
  check; the spec 374 stale criterion) and require the reviewer to flag them, plus near-miss specs it must pass. If it cannot, it does not ship.
- Cost: a review round is several model calls. Row 399's two worker attempts cost about $0.22 and $0.22; a review at the local model's price is cheaper than
  one wasted gate (about 25 minutes of build and test) but must be measured per round, with the per-run ceiling already in place (`max_micro_usd`).

## 8. Measurements before building (a row each, in this order)

1. Corpus: 3 known-bad specs from this session and 3 near-misses; the reviewer must flag all 3 and pass the 3 (the control).
2. Does the local model's verdict token parse every time on 30 inputs? (same harness as the deliberation note)
3. Does a second run at a different seed add findings? (decides item 5)
4. Cost per round and rounds to converge on the corpus. Decide the cap from the data, not from EGM's 5.
5. For the drift report: run it by hand on 5 documents and count true drifts against a human read, before scheduling anything.
