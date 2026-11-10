# Spec: 83 — the pii-memory `lanes` tests are hermetic (N1: a fresh tree fails the gate)

**Project**: <repo>
**Phase**: 045 re-review, candidate 1 — the reviewer's "single next most valuable change" (with 84).

## Context
`crates/pii-memory/tests/lanes.rs` has its own `tmp_root()` at `:23-25` which DOES
`create_dir_all(root.join("phase3"))`, but three tests bypass it and open hardcoded paths:

- `:123` → `concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp/phase3/lanes-degrade.redb")`
- `:158` → `.../_tmp/phase3/mem-hit-text.redb`
- `:188` → `.../_tmp/phase3/graph-lane.redb`

On a tree where `crates/pii-memory/_tmp/phase3` does not exist, all three fail with
`No such file or directory` (`degrades_honestly_without_embedder`,
`graph_lane_recalls_doc_via_shared_concept`, `memory_hit_carries_doc_text`). The developer
tree only passes because that untracked directory already exists there — so the gate is
green on exactly one machine. Verified by the reviewer and reproduced by the orchestrator
(scratch export at `<scratch>/pii-review/rr-own-8sQT`, `rr-ab-CNHj`).

## Requirements
R1. All three tests create their own store under a directory they CREATE. Use the file's
    existing convention: `tempfile::Builder::new().prefix(...).tempdir_in(tmp_root().join("phase3"))`
    (or `tmp_root()` directly), never a bare hardcoded absolute path.
R2. No test may depend on another test having run first. After the change, running
    `cargo test -p pii-memory --test lanes` ALONE, in a tree with no `_tmp/` at all, passes.
R3. No behaviour change to what the tests assert (same assertions, same inputs, same crates).

## Exit Criteria
- [x] `[ "$(grep -c '"/_tmp/phase3' crates/pii-memory/tests/lanes.rs)" = 0 ]` (verified 0; the one remaining `_tmp/phase3` mention is the expect() message inside `tmp_root()`, which is the helper that CREATES the directory)
- [x] `[ "$(grep -c 'tempdir_in' crates/pii-memory/tests/lanes.rs)" -ge 3 ]` (verified 3)
- [ ] FRESH-TREE PROOF: `S=$(mktemp -d <scratch>/lanes-XXXX); git -C <repo> archive HEAD | tar -x -C $S; cp -r crates/pii-memory/tests/lanes.rs $S/crates/pii-memory/tests/; cd $S && CARGO_TARGET_DIR=$S/target cargo test -p pii-memory --test lanes` exits 0 and prints `test result: ok` with 0 failed — with NO `_tmp` anywhere under `$S/crates`
- [ ] `cargo test -p pii-memory` exits 0 (full crate, both in the export and in the repo tree)
- [ ] `cargo fmt --all --check` exits 0 and `cargo clippy -p pii-memory --all-targets -- -D warnings` exits 0
- [ ] the three named tests appear in the report's output with their pass status

## Guardrails
- Do NOT delete or loosen existing tests or assertions; this is a hermeticity fix.
- Do NOT change `crates/pii-memory/src/**` (production code) — tests only.
- Do NOT `rm -rf` anything; scratch under `<scratch>/` only, and leave it in place.
- Do NOT commit; leave the tree for the orchestrator, and report the fresh-tree command's
  exact output (the point of this spec is a measurement, not a claim).

## Orchestrator verification (2026-10-09)
- **Fresh-tree RED→GREEN measured by the orchestrator** in its own export
  (`<scratch>/lanes-verify-FYuQ`, confirmed "no _tmp in export"):
  - HEAD's `lanes.rs` → `degrades_honestly_without_embedder`, `graph_lane_recalls_doc_via_shared_concept`,
    `memory_hit_carries_doc_text` **FAILED** with
    `memstore: open .../_tmp/phase3/{lanes-degrade,graph-lane,mem-hit-text}.redb:
    I/O error: No such file or directory (os error 2)`.
  - Working-tree `lanes.rs` in the same export → `test result: ok. 8 passed; 0 failed`.
  This is N1 reproduced and closed by measurement, not by report.
- Full `quality-gate.sh` ALL PASS in the repo tree (fmt / clippy -D warnings / tests / doctests /
  lint-files / reachability-check), no lint-files warning.
