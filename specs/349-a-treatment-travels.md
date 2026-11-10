# Spec: A treatment travels — the medicine one installation makes, another can take

## Overview

pii already records a healing attempt (spec 26): `signature_hash` for the failure, `patch_hash` for the fix,
`gate_before` / `gate_after` for the evidence, `decision` for who approved it, `attempts` for the history. And it
already detects the failure (spec 150), opens a case, fixes it on a branch and gates it (spec 151's harness, proven).

What it cannot do is the thing that makes self-improvement worth anything: **a cure found on one installation cannot
leave that installation.** Every install that hits the same bug pays for the same diagnosis, and the fix a machine
learned for itself dies with that machine's state directory.

This spec is the transport: a **treatment bundle** — the reproducer, the patch, and the evidence — that another
installation can import, verify, apply, and prove for itself. The bundle is a file: sharing today is `scp` or a git
branch; the GitHub/PR/specs-as-issues path is a *transport* for the same format, not a new mechanism.

**Project**: <repo>
**Phase**: self-healing — the step after detect-and-escalate (spec 07 Slices -1/0), and the first of the slices that
spec explicitly deferred ("patching, worktrees, git tools, or PR tools").

## Verified premises (measured 2026-11-02)

- `crates/pii-store/src/heal.rs` — `HealAttempt` carries `signature_hash`, `patch_hash: Option<String>`,
  `gate_before`, `gate_after`, `decision`, `attempts`, `state: HealState`. The record of a cure exists.
- `pii incident`-shaped fingerprints are stable strings (`refusal-repeated:channel.poll.failed`,
  `observability-lost:<run>`), which is what a treatment must be keyed by.
- Spec 151's harness proves an incident becomes a brief, a fix, and a green gate on a branch, with `master` unmoved.
- A patch without its reproducer cannot be trusted: the 2026-11-02 work found four "green" changes that proved
  nothing (a check that never ran, a helper nothing called, an impossible timing), so evidence travels WITH the fix
  or it does not travel at all.

## Requirements

### Functional

1. **`pii treat export <signature> --out <path>`** writes one bundle (a tar, or a directory, one format only):
   `manifest.json` + `repro.<ext>` + `fix.patch`. The manifest carries `format_version`, `signature`,
   `pii_version`, `base_revision` (the commit the fix was made against), `patch_sha256`, `repro_sha256`,
   `gate_before`, `gate_after`, `created_ms`.
   It **refuses** (exit 2, naming which field is missing) when the attempt has no `patch_hash`, no `gate_after`, or no
   reproducer: an unproven fix is not a treatment, and exporting it would spread a guess.
2. **`pii treat import <path>`** verifies before it records anything, and says which check failed:
   - `patch_sha256` and `repro_sha256` match the bytes in the bundle;
   - `format_version` is one this binary knows;
   - `base_revision` is a **commit in this repository** (ancestor of HEAD or HEAD itself) — a bundle built on someone
     else's history must say so rather than apply by luck;
   It then records the treatment in the local heal store as state `imported`, with its provenance (where it came
   from, when, which revision).
3. **`pii treat apply <signature>`** is the only act that touches the tree, and it is careful:
   - `git apply --check` first; a patch that does not apply cleanly is refused **and the tree is untouched**;
   - the reproducer must **fail before** the patch and **pass after** it — that pair is the proof the medicine works
     here, not merely that it applied. If it does not flip, the patch is reverted (`git apply -R`, tree restored
     byte-identically) and the treatment is recorded as `not_a_treatment` with the output of both runs.
   - the result is recorded locally as `applied` / `verified` with `gate_after` from THIS machine.
4. **`pii treat list`** shows one line per treatment: signature, base revision, state, whether it would apply
   cleanly here right now (a fresh `--check`, not a remembered answer), and the evidence it carries.
5. **No network, no push.** The bundle is a file. Nothing in pii contacts a remote registry; an operator moves the
   file, or a later transport does (git branch, PR, issue). This keeps the format useful in an air-gapped
   installation and keeps the decision about what a machine accepts in the operator's hands.
6. **Applying an imported treatment needs the approval the store already models** (`ApprovalDecision`), unless the
   operator passes an explicit `--yes` for a bundle whose signature matches an incident this machine opened itself.

### Non-Functional

- One binary: the logic lives in the `pii` CLI, not in a companion script.
- The bundle is deterministic: exporting the same attempt twice gives byte-identical files (no timestamps inside the
  patch, sorted JSON).
- A treatment never edits a spec, never touches `_trash`, and never commits.

## TDD Contract

| Test (`crates/pii-cli/tests/treat_travels.rs`) | Assertion |
|---|---|
| `export_refuses_a_fix_with_no_gate_after` | attempt with a patch but no proof → rc 2, message names `gate_after` |
| `CONTROL_export_writes_a_verifiable_bundle` | a complete attempt → bundle whose `patch_sha256` matches `sha256sum fix.patch` |
| `a_tampered_patch_is_refused_and_named` | one byte flipped → rc 2, names `patch_sha256`, nothing recorded |
| `a_bundle_from_another_base_is_refused_with_both_revisions` | unknown `base_revision` → rc 2, prints both |
| `an_imported_treatment_that_flips_its_reproducer_is_verified` | reproducer red before, green after → state `verified` |
| `CONTROL_a_treatment_that_does_not_flip_is_reverted` | reproducer green before → tree byte-identical after, state `not_a_treatment` |
| `a_patch_that_does_not_apply_leaves_the_tree_untouched` | conflicting patch → rc≠0, `git status --porcelain` unchanged |
| `export_is_deterministic` | two exports of one attempt are byte-identical |

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test treat_travels 2>&1 | grep -q '8 passed'`
- [ ] `bash scripts/checks/gates.sh`

## Guardrails

- Do NOT apply a patch without `--check` first.
- Do NOT believe a bundle's evidence: a treatment's proof is the reproducer flipping ON THIS MACHINE.
- Do NOT add a registry, a server, or a network call: the transport is a file.
- Do NOT export a fix whose gate is unproven, and do not let "it compiled" count as proof.
- Do NOT auto-apply from an untrusted source: the approval decision already exists in the heal store, so use it.
- Do NOT let a reverted patch leave a modified tree: the control asserts byte-identity.

## Files

- `crates/pii-cli/src/cmd_treat.rs`
- `crates/pii-cli/src/main.rs`
- `crates/pii-store/src/heal.rs`
- `crates/pii-cli/tests/treat_travels.rs`
- `_docs/05-operations/treatment-format.md`
