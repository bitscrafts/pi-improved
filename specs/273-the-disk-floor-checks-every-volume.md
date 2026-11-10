# Spec 273: the gate's disk floor checks every volume the gate writes to

## Overview
**Project**: <repo>
**Phase**: production — queue robustness. Twice (2026-10-24) a gate stalled or filled the host: once `/projects` hit 0 bytes free mid-gate (four steps failed with `No space left on device`), once cache moves filled the host root to 99 %. Both times the check looked at the wrong volume or at none.

`scripts/autonomy/cycle.sh:324-329` measures free space on `/projects` only and refuses under 20000 MB (`gate-skipped-disk`). But `_tmp/tools-gate/llvm-cov-target` and `_tmp/gate-target/llvm-cov-target` are SYMLINKS into `/tmp/gate-cache/` on the host root (`the host--vg-root`, a different volume, 25 GB free), so the gate writes tens of GB to a volume the check never reads.

## Verified Premises
- **CONFIRMED** (measured 2026-10-25): `cycle.sh:324` is `df -m --output=avail /projects`; the floor is 20000 MB; `_tmp/tools-gate/llvm-cov-target -> /tmp/gate-cache/llvm-cov-target` and `_tmp/gate-target/llvm-cov-target -> /tmp/gate-cache/gate-llvm-cov-target`; `/tmp/gate-cache` is 52 GB; `/projects` is `/dev/mapper/the host--vg-podman`, `/` is the `overlay` on `the host--vg-root`.
- **CONFIRMED** the outcome token `gate-skipped-disk` and `FREE_MB=` are what the queue's row note and `OUTCOME=` line carry; spec 268's `unlisted-changes.sh` is a model for a small standalone script with its own shell test.
- **NOT CLAIMED**: that 20000 MB is the right floor for the host root; the spec keeps one floor value and names it in one place. NOT CLAIMED that this frees any space: it only refuses early and names the volume. Deleting anything is the operator's decision and is out of scope.

## Requirements
### Functional
- **R1. `scripts/autonomy/disk-floor.sh <floor_mb> <path>...`** For each path: resolve it with `readlink -f` (a symlink is followed to the volume that really holds the data; a path that does not exist is reported `MISSING` and counts as a failure), read free MB with `df -m --output=avail,target` on the resolved path, and print one line `<path> -> <resolved> free=<MB>M mount=<target>`. Exit 0 when every path's free MB is `>= floor_mb`; otherwise print `LOW: <resolved> free=<MB>M floor=<floor_mb>M mount=<target>` for each failing path and exit 1. A non-numeric floor or no path exits 2 with a usage line. No `rm`, no `mv`, no write outside stdout.
- **R2. `cycle.sh` uses it.** Replace the single `df` test at `cycle.sh:324-325` by `disk-floor.sh 20000 /projects <every directory the gate writes>`, where that list is the real `CARGO_TARGET_DIR`s plus the resolved `llvm-cov-target` links: take it from ONE variable `GATE_WRITE_DIRS` defined in `scripts/autonomy/env.sh` (its default is the two `_tmp` target dirs and `/tmp`), so the list is stated once. The refusal keeps the token `gate-skipped-disk` and `last=gate-skipped-disk`, and its `say` line gains the failing mount from the script's `LOW:` output. Nothing else in `cycle.sh` changes.
- **R3. Tests, with controls.** New `scripts/autonomy/tests/disk-floor.test.sh`, in the style of `unlisted-changes.test.sh` (a `PASS`/`FAIL` line per case, exit non-zero on any FAIL), cases: (a) a real directory with a floor of 1 passes and prints `free=`; (b) the same directory with a floor of 99999999999 fails, prints `LOW:` naming the resolved path and exits 1; (c) a symlink to a directory on ANOTHER volume (`/tmp` vs `/projects` when they differ; the test reads both `df` mounts and prints `SKIP` with the reason when they are the same volume) reports the TARGET's mount, not the link's; (d) a missing path exits 1 with `MISSING`; (e) two paths, one failing: exit 1 and only the failing one carries `LOW:`; (f) no arguments and a non-numeric floor both exit 2; (g) CONTROL: the script contains no `rm`, `mv` or `>` redirect to a file (a `grep` over its own text); (h) the `cycle.sh` wiring: `grep` proves `cycle.sh` calls `disk-floor.sh` and no longer contains `df -m --output=avail /projects`, and `env.sh` defines `GATE_WRITE_DIRS`. Also run `bash -n` on all three scripts.

### Non-Functional
- Every file under the 450-line gate. No new dependency. Shell only. No Python in the image path.
- Do NOT change the floor value, the outcome token, or any step of the gate. Do NOT delete or move anything.

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/disk-floor.test.sh 2>&1 | tail -3 | grep -q "FAIL 0\|0 FAIL"`
- [ ] `bash scripts/autonomy/tests/disk-floor.test.sh 2>&1 | grep -c "^PASS" | grep -qE "^[0-9]+$"`
- [ ] `! bash scripts/autonomy/tests/disk-floor.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash -n scripts/autonomy/disk-floor.sh && bash -n scripts/autonomy/cycle.sh && bash -n scripts/autonomy/env.sh`
- [ ] `grep -q "disk-floor.sh" scripts/autonomy/cycle.sh`
- [ ] `! grep -q "df -m --output=avail /projects" scripts/autonomy/cycle.sh`
- [ ] `grep -q "GATE_WRITE_DIRS" scripts/autonomy/env.sh`
- [ ] `grep -q "gate-skipped-disk" scripts/autonomy/cycle.sh`
- [ ] `bash scripts/autonomy/tests/unlisted-changes.test.sh 2>&1 | grep -q "0 FAIL"`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT delete, move or truncate any file or directory. Do NOT change `20000`. Do NOT edit any Rust. Do NOT commit.

## Follow-ups (not this spec)
1. Caches in one bounded, self-pruning place — needs the operator's decision on what may be deleted (the 33 GB `_trash` and the `verify-*-target` dirs are candidates; nothing here touches them).
2. `gate-skipped-disk` parking a row with no retry once space returns.
