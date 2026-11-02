# Treatment bundle format (spec 349)

A **treatment** is one proven fix for one failure signature: the patch, the reproducer that showed the failure,
and the evidence. `pii treat` moves it between installations as a file. Nothing in pii contacts a network or a
remote; an operator (or a later transport such as a git branch or PR) moves the directory.

## Bundle layout (`format_version` 1)

A directory with exactly three files:

| File | Contents |
|---|---|
| `manifest.json` | sorted-key JSON: `format_version`, `signature`, `pii_version`, `base_revision`, `patch_sha256`, `repro_sha256`, `gate_before`, `gate_after`, `created_ms` |
| `fix.patch` | the patch, a `git apply`-able diff |
| `repro.sh` | the reproducer: a shell snippet, exit 0 = the failure is gone, non-zero = the failure is present |

`created_ms` is the attempt's last sighting, not the export time, so exporting one attempt twice gives
byte-identical files.

## Commands

- `pii treat export <signature> --out <dir>` — refuses (rc 2, naming the field) when the attempt has no
  `patch_hash`, no `gate_after`, no patch, or no reproducer. An unproven fix is not a treatment.
- `pii treat import <dir>` — verifies `format_version`, `patch_sha256`, `repro_sha256`, and that `base_revision`
  is an ancestor of (or equal to) HEAD in this repository, naming the failed check and both revisions. Records
  state `imported` with provenance (`treatment_source`, `treatment_base_revision`, `treatment_imported_ms`). It
  never copies the bundle's gate evidence as this machine's evidence.
- `pii treat apply <signature> [--yes]` — the only command that touches the tree:
  1. the approval gate: an `allow` decision in the heal store, or `--yes` for an incident this machine opened
     itself (its attempt has a local run history);
  2. `git apply --check` — a patch that does not apply is refused and the tree is untouched;
  3. the reproducer runs **before** the patch, then the patch is applied, then the reproducer runs **after**;
  4. flip (fail before, pass after) → state `verified`; no flip → `git apply -R` restores the tree and the state
     becomes `not_a_treatment`. Both runs' output is kept in the attempt.
- `pii treat list` — one line per treatment, with a fresh `--check` against the current tree.

## Heal store fields used

Treatment bytes and provenance live in the attempt's `extra` bag (`treatment_patch`, `treatment_reproducer`,
`treatment_base_revision`, `treatment_source`, `treatment_imported_ms`, `treatment_apply_before`,
`treatment_apply_after`). New `HealState` values: `imported`, `verified`, `not_a_treatment`.
