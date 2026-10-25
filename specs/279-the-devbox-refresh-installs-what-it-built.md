# Spec 279: the devbox refresh installs the binary it just built

## Overview
**Project**: /projects/pi-improved
**Phase**: production — deploy identity. Supersedes spec 278 (closed unimplemented: its premise was measured false, see its Closure section).

`modules/devbox/daemon.sh refresh` builds the `pii` binary inside the devbox container and then installs a binary into `_tmp/devbox-test/bin/pii`. It installs the WRONG one. Measured 2026-10-25 in the container:

- the container sets `CARGO_TARGET_DIR=/work-target` (`modules/devbox/compose.yaml:49`), so `cargo build` writes `/work-target/debug/pii`;
- `daemon.sh:34` copies `/projects/pi-improved/target/debug/pii`, a different file: a stale host-side build from 15:38 (the dirty tree of an earlier worker);
- the daemon's binary has sha256 `fd74ad2c716d…`, byte-identical to that stale file; the freshly built `/work-target/debug/pii` (16:36, sha `044152aa9cc1…`) carries the label `b8358f7519f5`, which IS the HEAD it was built from.

So the label never lied: `build.rs` is correct, and the daemon was running old code under a truthful old label. Every `refresh` reports success (`cp` exits 0) while installing nothing new.

## Verified Premises
- **CONFIRMED** (read and measured 2026-10-25) `daemon.sh:23` (first-install fallback) and `daemon.sh:34` (refresh) both copy from the literal `/projects/pi-improved/target/debug/pii`; the container's `CARGO_TARGET_DIR` is `/work-target`; the two files differ (sha above); the daemon's binary equals the literal-path file.
- **CONFIRMED** the remote part of `daemon.sh` runs inside `ssh "$HOST" bash -s <<'REMOTE'` with `X() { podman exec -w /projects/pi-improved pii-devbox "$@"; }`, so the container's own environment decides the target dir.
- **MEASURE FIRST.** (1) Read `modules/devbox/check.sh` and the gate step `devbox-check` (`quality-gate.sh`, `gate-steps.txt`): which files in `modules/devbox/` does it enumerate or require? A new file must not break it. If a new script needs a registration line, add exactly that line and say so. (2) Whether `modules/devbox/` already has a test directory convention; if not, put the new test at `scripts/autonomy/tests/devbox-install-binary.test.sh` beside the other shell tests.
- **NOT CLAIMED**: that the running daemon is refreshed by this spec. Landing the fix changes nothing in the running container; a `daemon.sh refresh` afterwards is a separate deliberate act with no worker in flight.

## Requirements
### Functional
- **R1. `modules/devbox/install-binary.sh <target_dir> <dest>`.** Exactly two arguments, else exit 2 with a usage line. The source is `<target_dir>/debug/pii` and nothing else: if it is missing or not executable, exit 1 naming that exact path and touch nothing. Otherwise copy it to `<dest>.new`, compare `sha256sum` of source and `<dest>.new`; on a mismatch remove only `<dest>.new` and exit 1 naming both digests (first 12 hex); on a match `mv -f <dest>.new <dest>` and print `installed <12 hex> from <source>`. It never reads or writes any other path, never deletes anything but its own `.new`, and takes no path from the environment.
- **R2. `daemon.sh` uses it.** Replace BOTH copy lines (`daemon.sh:23` and `:34`) by a call to `install-binary.sh` with the container's real target dir as first argument: the value of `${CARGO_TARGET_DIR:-target}` evaluated INSIDE the container (relative `target` is resolved from `-w /projects/pi-improved`), and `$PII` as the destination. A failing install makes `refresh` and `up` exit non-zero before the daemon is restarted. After the daemon answers, `refresh` prints one more line, `daemon binary sha <12 hex>`, read with `sha256sum $PII` inside the container, so an operator can compare it to the `installed …` line. The `pkill`, the build command, `start_daemon`'s env and every other line stay exactly as they are.
- **R3. Tests, with controls, reading the live path.** New shell test, style of `scripts/autonomy/tests/cycle-disk-wiring.test.sh` (one `ok <name>` / `FAIL <name>` line each, a final `<N> ok, <M> FAIL` line, exit 0 only when `M` = 0, scratch under `_tmp/` removed by its own `trap`). Cases: (a) THE BUG: a scratch `root/target/debug/pii` (decoy, content `OLD`) AND a scratch `T/debug/pii` (content `NEW`): `install-binary.sh T dest` installs `NEW` (the dest's bytes equal `T/debug/pii`, not the decoy) and prints `installed` with the digest of `NEW`; (b) CONTROL: a missing `T/debug/pii` exits 1, names the path, and leaves an existing `dest` byte-identical; (c) CONTROL: a `cp` shim earlier on `PATH` that truncates its copy makes the script exit 1, remove `dest.new`, and leave `dest` unchanged; (d) CONTROL: wrong argument counts exit 2; (e) CONTROL: a good install over an existing different `dest` replaces it; (f) THE WIRING, read from the live text: extract from `daemon.sh` the non-comment line(s) that call `install-binary.sh` at run time, assert there are exactly two call sites and that NEITHER contains the literal `/projects/pi-improved/target/debug/pii`, and evaluate the target-dir argument text with `CARGO_TARGET_DIR=/planted` (must yield `/planted`) and with it unset (must yield `target`): never a `grep` for a variable name alone; (g) CONTROL: the same wiring check reddens a scratch copy of `daemon.sh` with the literal path restored. Also run `bash -n` on both scripts.

### Non-Functional
- Every file under the 450-line gate. Shell only. No change to `compose.yaml`, `Containerfile`, `build.rs`, any Rust, `quality-gate.sh`.
- The new scripts delete nothing but `<dest>.new`.

## Exit Criteria
- [ ] `bash scripts/autonomy/tests/devbox-install-binary.test.sh 2>&1 | tail -1 | grep -qE "^[0-9]+ ok, 0 FAIL"`
- [ ] `! bash scripts/autonomy/tests/devbox-install-binary.test.sh 2>&1 | grep -q "^FAIL"`
- [ ] `bash -n modules/devbox/install-binary.sh && bash -n modules/devbox/daemon.sh`
- [ ] `grep -q "install-binary.sh" modules/devbox/daemon.sh`
- [ ] `test "$(grep -v '^[[:space:]]*#' modules/devbox/daemon.sh | grep -c 'cp /projects/pi-improved/target/debug/pii')" = 0`
- [ ] `grep -q "daemon binary sha" modules/devbox/daemon.sh`
- [ ] `bash modules/devbox/check.sh 2>&1 | tail -1 | grep -qiE "ok|pass|0 fail"`
- [ ] `cargo run -q -p xtask -- lint-files 2>&1 | tail -1 | grep -q "lint-files:"`

## Guardrails
- Do NOT run `daemon.sh refresh`, `up`, `down` or any `ssh`/`podman` command in a test: the wiring is read from text, the helper runs on scratch files. Do NOT touch `_tmp/devbox-test/`. Do NOT edit Rust or `/root/.pi`. Do NOT commit.
- If `devbox-check` rejects a new file, STOP and report what it enumerates.

## Follow-ups (not this spec)
1. After it lands, `daemon.sh refresh` with no worker in flight, and compare the two printed digests.
2. A deploy freshness rule that compares the daemon's sha to the built artefact instead of reading a label.
