# Spec: the book explains how a call passes data

## Overview

The operator asked how one graph passes information to another, and the answer was not in either
book. It has been drafted into `_docs/manuals/src/arch-dev/src/chapters/05-composition.tex`
(section "How a call passes data"), and this row is what makes it a finished chapter rather than a
draft: it must compile, render, and name only paths and verbs that exist.

The section documents the three moves — `[node.in]` computing the callee's entry inputs in the
caller's context, the splice under `<call>__<node>` ids, and the `$.nodes.<call>` →
`$.nodes.<call>__<exit>` aliasing — plus the two rules that travel with the boundary (a call returns
on its exit's ports; a call only narrows the host's ceiling) and the trap that made a real graph
ship with an empty brief.

**Project**: /projects/pi-improved
**Phase**: documentation

## Requirements

- **R1 — it compiles.** The architecture book builds through the LaTeX container with no error, and
  the new section appears in the table of contents.
- **R2 — the figure renders.** The three-move diagram is legible at page width: no label collides
  with a node, no edge crosses a label, and the caption is below the picture.
- **R3 — every claim is checkable.** The paths named in prose (`fragment/call.rs`,
  `fragment/call_output.rs`) exist; the syntax shown matches a shipped graph; the `in` keys are
  described as they are, not as they were hoped to be.
- **R4 — the trap is stated with its evidence.** The `$.nodes.inventory.payload.stdout` example
  names the file it happened in and says that no test caught it, because no test ran the graph.
- **R5 — the two books stay consistent.** The user book's graph-library chapter points at this
  section rather than duplicating it.

## Exit Criteria

- [ ] `test -f _docs/manuals/pii-arch-dev.pdf`
- [ ] `grep -q 'How a call passes data' _docs/manuals/src/arch-dev/src/chapters/05-composition.tex`
- [ ] `grep -q 'nodes.inventory.payload.stdout' _docs/manuals/src/arch-dev/src/chapters/05-composition.tex`
- [ ] `grep -q 'fragment/call_output.rs' _docs/manuals/src/arch-dev/src/chapters/05-composition.tex`

## Guardrails

- Do NOT describe the mechanism from memory. Three moves are documented because the loader was read:
  input keys become `<call>__in__<key>`, ids are namespaced, and the alias is a rewrite.
- Do NOT paste the graph as a listing without saying which file it comes from.
- Do NOT claim the section renders until a page image has been inspected.

## Files

- `_docs/manuals/src/arch-dev/src/chapters/05-composition.tex`
- `_docs/manuals/pii-arch-dev.pdf`
