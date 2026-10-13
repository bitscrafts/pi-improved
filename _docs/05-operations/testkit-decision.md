# Decision: no `pii-testkit` crate (spec 147)

Backlog rule (`open-items.md` §6): a testkit crate is worth it only if a second or third crate duplicates
the helpers. Checked, not assumed.

## Census

Re-run: `bash scripts/checks/test-helper-census.sh`. It counts test fns that *reimplement* a shared concern
(scratch/tmp dirs, store fixtures, fake clocks, event capture). A scratch fn that delegates to the existing
shared `pii_store::testing::tmp_scratch` is a thin wrapper and is counted in `wrap`, not as duplication.

| crate | scratch | store | clock | capture | total | wrap |
|---|---|---|---|---|---|---|
| pii-channels | 2 | 0 | 0 | 0 | 2 | 0 |
| pii-cli | 2 | 0 | 0 | 0 | 2 | 0 |
| pii-memory | 0 | 1 | 0 | 0 | 1 | 2 |
| pii-server | 16 | 0 | 0 | 0 | 16 | 10 |
| pii-store (owner, excluded) | 7 | 0 | 0 | 0 | 7 | 2 |

Crates not listed have no such helpers. Fake clock (`pii_core::FixedClock`) and event capture
(`pii_observe::Recorder`) already live in one place each; no crate re-implements them.

## Rule

Build the crate **iff** at least two crates outside `pii-store` each duplicate at least three helpers.

## Outcome: NOT WORTH IT

Crates outside `pii-store` with >= 3 duplicated helpers: **1** (`pii-server`, 16). Needed: 2. The rule does
not fire, so no crate is created and no helper is moved. The scratch-dir concern already has one home,
`pii_store::testing::tmp_scratch`, which `pii-memory` and 10 `pii-server` tests already use.

## Residual duplication left alone, and why

- `pii-server`'s 16 own `scratch` fns (mostly returning `PathBuf`, a few `TempDir`): each is local to one test
  file and used by that file only; migrating them is a mechanical clean-up toward the existing
  `tmp_scratch`, not a reason for a new crate.
- `pii-channels` (2) and `pii-cli` (2) `scratch` fns: below the threshold of 3, single-file use.
- `pii-memory`'s `fresh_store`: used in one test file, one occurrence.
