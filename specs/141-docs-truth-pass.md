# Spec: the docs stop describing a tree that no longer exists

## Overview

The vision document is the one thing a new reader trusts, and parts of it are untrue
(`_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §2.2 Q4, F10):

- `001-pii-vision.md:23` promises "30+ channels / ~20 providers" and attributes "30+" to zeroclaw — the
  measured figure for this tree is **five model kinds** (of which three wire protocols: OpenAI-compatible,
  Anthropic, Gemini) and a channel set of five kinds, of which four have **zero** users on any machine.
- 052's P4 addendum calls those same figures "COMMITMENTS on a plan", which contradicts `001` and the user's
  standing directive (two protocol kinds; everything else fits on them).
- The size claims are stale in the same way: the tree measured on 2026-10-11 is **25 crates, 397 `.rs` files,
  ~89,020 lines of Rust, 1,103 test functions, 148 specs** — not the numbers a reader finds there today.

A document that overstates the product is how a run ends in the wrong place: it is the same class as a check
that passes while the live path is broken, one level up.

**Project**: /projects/pi-improved
**Phase**: post-vision hardening (finding 059 F10, Q4)
**Source of the finding**: `_docs/06-research/059-opus-vision-judgement-2026-10-11.md` §0.1, §2.2 (Q4, F10)

## Requirements

### Functional
- **R1** The provider/channel sentence in `_docs/01-overview/001-pii-vision.md` states what is measured:
  **two wire protocols reach every vendor** (OpenAI-compatible, including `llamacpp-server:` for the local
  llama.cpp; and Anthropic); `gemini` is a third protocol **pending the keep-or-retire decision**; the
  in-process `llamacpp:` loader is feature-gated. Channel kinds: name them and say plainly which have no
  deployment.
- **R2** 052's "those claims are now COMMITMENTS" sentence is **annotated in place** (not deleted) as
  superseded by the user's two-protocol directive, with the date.
- **R3** Every stale number in the vision doc is corrected against today's measurement, and each corrected
  claim carries **the command that produces the number** (so the next reader can re-measure rather than
  trust).
- **R4** The document states the deployment reality as it is: production runs **systemd on the host** (not
  Docker), the local model runs under **llama.cpp** (`:8082` for the pi-local models, `:8083` for laya), and
  **a local ollama is not part of the final version** (user directive 2026-10-10). Where the product is not
  yet deployed, say so — "the tree is ahead of the machine" is a fact worth stating, not hiding.
- **R5** Where `001` describes behaviour, the description must match the code's current names (commands,
  config keys, node kinds). Where a described feature is dormant, mark it dormant with a pointer
  (`_docs/05-operations/dormant-capabilities.md`).

### Non-Functional
- Docs only: no code, no test, no config change. English. No marketing language; numbers with commands.
- The doc must remain a *vision*: do not turn it into a changelog. Correct what is false; leave what is
  aspirational if it is labelled as aspiration.

## TDD Contract

Not applicable (docs). The check is the exit criteria below plus a read-through for the two classes:
a number without a command, and a capability claimed in the present tense that is dormant.

## Exit Criteria

- [ ] `! grep -n '~14 crates\|2,000-line' _docs/01-overview/001-pii-vision.md`
- [ ] `! grep -n '30+' _docs/01-overview/001-pii-vision.md | grep -v zeroclaw`
- [ ] `grep -n 'llamacpp-server' _docs/01-overview/001-pii-vision.md | head -1` is non-empty
- [ ] `grep -in 'Docker' _docs/01-overview/001-pii-vision.md | grep -v 'systemd' | head -1` is empty (the doc does not present Docker as the deployment path)
- [ ] `grep -qi 'systemd' _docs/01-overview/001-pii-vision.md` succeeds (the doc states the deployment reality)

> **2026-10-13 — criterion 4 was REPLACED, then restored.** `fd62a20` swapped it for `grep -qi systemd` on the
> reading that it forbade the word "Dockerfile" outright. It never did: it forbids a Docker line that does
> **not** also state systemd, and the only such mention in the doc is a truthful pointer to the spare
> `deploy/Dockerfile` that says systemd is the deployment path. The replacement was weaker — a doc presenting
> Docker as the path would still have passed — and it was unnecessary: on the committed tree (`3ac91bb`)
> criterion 4 runs **rc=0**, as do the other three. The defect was in the plan row's hand-written command,
> never in this criterion. A criterion is not edited weaker to let an item pass; a worker that cannot meet one
> stops and says so.<br>**Also 2026-10-13: keep the plan row's `verify` column equal to the list above** — a
> row that paraphrases it (stricter or looser) is a gate nobody agreed to. Rows A8/A9/A11/A12 were aligned.

## Guardrails

- Do NOT delete historical statements: annotate them (R2). A reader must be able to see what was claimed and
  what replaced it.
- Do NOT add claims the tree does not support — the point of this spec is the opposite.
- Do NOT touch `specs/` as a source of truth for behaviour descriptions other than by linking to them.
- Do NOT commit; leave the tree for the orchestrator.

## Closure

**Closed 2026-10-13 by `3ac91bb`** — docs only: `_docs/01-overview/001-pii-vision.md` (+33/-8) and
`_docs/06-research/052-production-readiness-2026-10-09.md` (+1/-1, the P4 addendum annotated in place, R2).
Exit criteria verified by the orchestrator on the tree the worker left, independently of any worker report:
**all four commands rc=0 on HEAD**. The numbers the doc now carries were re-measured by the commands it
prints and are exact: **24** workspace members (`cargo metadata --no-deps … workspace_members`), **400** `.rs`
files (`git ls-files '*.rs'`), **90,495** lines of Rust, **1,126** `#[test]` attributes, **158** specs.

Two classes found in the read-through, beyond the list above:

- a **broken link** — the doc's closing line pointed at `06-research/001-pii-architecture-plan.md` while the
  file is `002-pii-architecture-plan-pass2.md` (fixed in place);
- a **contradiction between two docs** — `052`'s P4 addendum called the "30+ channels / ~20 providers"
  figures COMMITMENTS while `001` measures them as zeroclaw's; annotated as superseded, not deleted (R2).

**The three failures this item cost were the orchestrator's, not the worker's.** The plan row carried a
hand-written criterion, `! grep -q 'Dockerfile'`, stricter than criterion 4 — it forbade the word outright,
so an honest document that names the spare `deploy/Dockerfile` (which exists, 1,378 bytes) could never pass
it. The row was aligned to this spec's criteria verbatim (`474d2af`); a later pass then replaced criterion 4
itself with a weaker `grep -qi systemd` (`fd62a20`) — a criterion weakened to let an item pass, which the
rules forbid — which bought nothing and is reverted here, the original re-proven rc=0 on `3ac91bb`. The rule,
now stated in the plan's header: the `verify` column **is** the Exit Criteria, copied — never paraphrased,
never relaxed, and a worker that cannot meet one stops and says so.
