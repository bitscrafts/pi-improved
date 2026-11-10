# Spec: Disk is owned, bounded and visible

**Project**: <repo> · **Phase**: 0.2.1 · **Measured**: 2026-11-03, after a full clean.

## Why this exists

The container's `/` is a 288 GB overlay that reached **16 GB free (95%)** while `/projects` — a separate 594 GB
podman volume — had **147 GB free**. The gate refuses to run below a 12 GB floor on `/`, so work stopped
(`row 358: last=gate-skipped-disk`) and the two things an operator would try first — `cargo clean` on the repo —
freed 152 GB on the volume that was never short of space.

Four measured mechanisms fill the disk, and **no component owns their total**:

| mechanism | measured | why it grows |
|---|---|---|
| the gate's warm cache `_tmp/gate-target` | **105.8 GiB**, 184,639 files, removed by one `cargo clean` | one persistent target holds every crate/feature/profile ever built; a bigger cache is faster, so nothing ever bounds it |
| one-off experiment targets | `_tmp/tools-gate` 6.7 G · `_tmp/verify-a102-target` 3.6 G · `_tmp/test-gate-repro` 3.3 G · `_tmp/verify-244-main-targ` 2.6 G · `/tmp/gate-cache/gate-llvm-cov-target` **58 G** coverage | each experiment creates a full target and nobody removes it; the oldest are from 21–28 Oct and finished long ago |
| trash | `<workspace>/_trash` 57 G · `_tmp/_trash` 48 G · `/tmp/_trash-podman` 33 G · `/tmp/_trash-v234` 7 G ≈ **145 G** | 30-day retention keeps a month of quarantines and moves |
| gate exports | kept on failure, removed on success (retention landed 2026-11-03) | a failing gate keeps a full repo copy *and* its own test target — and gate failures are common |

The mismatch that hid all of it: **the floor is measured on `/`, but the big artefacts land on `/projects`.** Freeing
the volume looks like progress and is worth nothing.

## Requirements

- **R1** One command prints the artefacts and their sizes, per filesystem:
  `pii disk` (or `cargo xtask disk`) lists every known artefact root — gate cache, ad-hoc targets, exports, trash,
  cargo caches — with bytes, age and the filesystem it lives on, and prints free space for **both** `/` and the
  volume holding the repository.
- **R2** The gate cache has a ceiling and is trimmed, never silently grown: when `_tmp/gate-target` exceeds the
  configured `max_gb` (default 40), the gate trims it before building and says so in its output.
- **R3** Ad-hoc targets live under one owner with a TTL: `_tmp/targets/<purpose>` older than `ttl_days`
  (default 7) is removed by the same command (`--gc`), and a target created outside that root is reported as
  unowned rather than ignored.
- **R4** The disk floor is evaluated on the filesystem that will receive the artefacts, and the refusal names it:
  a message that says "12.0 GB floor on /, 4.1 GB free" when `/projects` has 147 GB is a lie by omission.
- **R5** Retention removes a failed gate's export after `export_ttl_hours` (default 24) unless a gate owns it, so a
  weekend of failing gates cannot cost tens of gigabytes.

## Files

- `xtask/src/disk.rs`
- `xtask/src/main.rs`
- `xtask/src/fresh_gate.rs`
- `crates/pii-cli/src/cmd_disk.rs`
- `crates/pii-cli/src/main.rs`
- `crates/pii-cli/tests/disk_report.rs`

## Exit Criteria

- [ ] `cargo test -q -p pii-cli --test disk_report 2>&1 | grep -q 'test result: ok'`
- [ ] `cargo run -q -p xtask -- disk 2>&1 | grep -q 'filesystem'`
- [ ] `cargo run -q -p xtask -- disk 2>&1 | grep -q 'gate-target'`
- [ ] `grep -q 'max_gb' xtask/src/fresh_gate.rs`
- [ ] `grep -q 'ttl_days' xtask/src/disk.rs`

## Guardrails

- Do NOT delete a target, export or trash directory that another process is using; the gate already learned this
  (retention removed a running gate's export, 2026-11-03).
- Do NOT remove anything outside the artefact roots this spec names.
- Do NOT lower the 12 GB floor to make a gate pass.
- Do NOT trim the cache while a build is running: check for a live cargo/rustc before any removal.
