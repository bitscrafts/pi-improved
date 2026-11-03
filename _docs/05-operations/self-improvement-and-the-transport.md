# Self-improvement, self-healing, and how a cure travels

Recorded 2026-11-02, from the operator's own words:

> "pii codebase should be in a github repo and we should act on via PRs, in the future the specs should be created in
> github! any pii installation should be able (when finding a bug) to be able to fix it and provide a path to all
> other installations to use it and test it! we gonna have a self improvement path that will allow pii to debug
> itself and share the medicine! we are on a new perspective view of the future!"

This is the direction, written down so it is not re-derived in six weeks. Note what is **not** authorised yet:
nothing is pushed to any remote ("no pushing!!!!"), and no repository is created. The transport stays a file until
the operator says otherwise.

## The three layers, and what already exists

| layer | what it means | what exists today |
|---|---|---|
| **detect** | the failure becomes a record, no model involved | spec 150: `pii incidents scan` over the daemon's own log; rules fixed against real logs (346 added `observability-lost`) |
| **heal** | a failure becomes a proven fix on a branch, never on `master` | spec 151's harness: incident → brief → branch → gate, `master` unmoved; the heal store (spec 26) holds `signature_hash`, `patch_hash`, `gate_before/gate_after`, `decision`, `attempts` |
| **share the medicine** | another installation takes the cure, verifies it, and proves it locally | **missing** — spec 349: the treatment bundle. Without it, every install pays for the same diagnosis and a cure dies with the machine that found it |

## Why the middle layer is not enough

A fix that cannot travel means:

- the same bug is diagnosed N times, once per installation;
- the evidence that a fix works is trapped where it was produced;
- "self-improvement" degrades into "self-repair", and the operator becomes the courier.

## The transport, in order of ambition

1. **Today — a file.** A treatment bundle (spec 349): manifest + reproducer + patch, with the sha256 of both and the
   base revision. Verified on import, proven on apply by the reproducer flipping red→green on the receiving machine.
   No network, no registry: an air-gapped installation is a first-class citizen.
2. **Next — a git branch.** The bundle rides a branch; the receiver fetches the branch and applies the same bundle.
   The format does not change; only the way it arrives.
3. **Then — a PR, with the spec as an issue.** The repo gets a remote, work happens on branches, and the gate runs in
   CI so a PR carries its own evidence. Specs become issues, and a closed issue is a treatment with a link.
4. **Later — pii opens its own PRs.** The loop that already produces a branch and a green gate hands that branch to a
   forge instead of to an operator.

Each step is *transport*, not a new mechanism — which is why the bundle format is designed first and the forge second.

## From an incident to a case (spec 350)

The host reports an incident; the workshop opens a **case** for it without a person. The case ladder, in
`_tmp/autonomy/incident-cases.tsv` (append-only; the last line for a case id is its state):

1. **`new`** — a fingerprint the ledger has not seen opens a case with its evidence. Silent otherwise, capped at five per
   run (oldest first). A case is **not a row**: `cycle.sh` dispatches only `plan.tsv`.
2. **`briefed`** — `incident-cases.sh --brief <case_id>` writes `specs/auto-<case_id>.md` with the evidence and a
   `## TODO criteria` section. The criteria are written by a person or the skill lane; a machine cannot invent a
   criterion that can fail.
3. **`queued`** — a row exists in `plan.tsv` for the briefed spec.
4. **`closed`** — a treatment landed. If the fingerprint recurs after this, it opens `<fp>#2`: a cure that did not hold
   is its own finding.

## The rule that keeps this honest

A treatment's proof is **the reproducer flipping on the machine that receives it**, not the evidence attached to the
bundle. Any installation can lie, any bundle can be stale, and a patch that applies cleanly can still fix nothing —
measured four times on 2026-11-02 alone, in four different forms (a check that never ran, a helper nothing called, an
impossible timing, a criterion tied to a scratch directory).

So: `git apply --check` before anything, red→green before "verified", and the tree byte-identical after any revert.

## What the operator decides, and when

- **Transport**: when a remote exists at all, and whether it is public. Until then, files.
- **Acceptance**: an imported treatment is applied only with the approval the heal store already models.
- **Publishing**: which treatments leave this machine, because a bundle carries a patch and a reproducer — evidence
  about this installation, not only about the bug.
