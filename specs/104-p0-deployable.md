# Spec: 104 — P0: make it deployable

**Project**: /projects/pi-improved
**Phase**: P0 of the road to production (`_docs/06-research/052-production-readiness-2026-10-09.md` §8).
**Goal**: a target host where a user starts pii, it keeps running, and they can see why it died.

## Measured facts this spec starts from (do not re-derive; correct me if you measure otherwise)
- `pii daemon --help` → `error: unknown flag '--help'`; only usage strings exist.
- No `Dockerfile`, no systemd unit, no release build has ever been produced; `target/debug/pii` is
  `ELF 64-bit LSB pie … dynamically linked` (libgcc_s, libm, libc).
- `rustup target list --installed` → `wasm32-unknown-unknown`, `x86_64-unknown-linux-gnu`. The musl
  target is NOT installed and no musl linker is present.
- Health identity already exists: `pii health --json` reports build/config/stores/mounts/counts (spec 99).

## Requirements
R1. **Settle the static-vs-dynamic question with evidence, not prose.** Attempt the real thing:
    `rustup target add x86_64-unknown-linux-musl` then
    `cargo build --release --target x86_64-unknown-linux-musl -p pii-cli`.
    - If it builds: paste `file target/x86_64-unknown-linux-musl/release/pii` (must say *statically
      linked*) and `ldd` refusing it.
    - If it cannot (no network, no musl linker): paste the exact error and what is missing, then take the
      documented-decision path in R2. **Do not fake success and do not leave the question open.**
R2. **Make the documentation and the artefact agree.** `_docs/01-overview/001-pii-vision.md` claims a
    "single static Rust binary"; reality is four binaries, dynamically linked. Either the release build
    makes the claim true, or the claim is corrected in `001` (with the `file`/`ldd` evidence and a pointer
    to this spec's result). A stale vision claim next to a contradicting artefact is the defect.
R3. **`pii daemon --help` must work**: exit 0, list the real flags (store path, config path, runtime dir,
    sandbox profile, log file, log rotation, trajectory dir if present) with one-line help each. Unknown
    flags must still be refused with a non-zero exit.
R4. **A production config sample**, checked in, with every key documented (store, workspace scope, mounts,
    sandbox profile, channels, worker, log file + rotation, trajectory dir) and a note on which keys are
    required vs defaulted.
R5. **Log to a file, with rotation.** A config key/flag (`log_file`, `log_rotate_bytes`, `log_keep_files`
    — name them as you see fit and document them), size-based rotation, bounded keep count, and the
    daemon's existing events written to it (the contract's events, not new ad-hoc prints — the
    `lint-observability` budget will hold you to that). Default OFF so no existing behaviour changes.
R6. **A probe a supervisor can use**: `pii health --json` (already present) must answer while the daemon
    is up and must exit **non-zero** when the daemon is not reachable. Paste both exits.
R7. **A real deploy artefact**: a `Dockerfile` **and/or** a systemd unit that builds/starts the daemon with
    a declared store, runtime dir and log file. Then prove the run: start it, probe it, `kill -9` it,
    restart it, and show the store is intact (e.g. the run/approval counts from `pii health --json` are the
    same before and after, or the same run can be read back).

## Exit Criteria
- [ ] R1/R2 resolved and pasted: either `file` on the release binary says *statically linked*, or the
      exact blocking error plus the corrected `001` claim (quote the before/after lines)
- [ ] `pii daemon --help` exits 0 and lists every real flag; an unknown flag still exits non-zero (paste both)
- [ ] the config sample is checked in and every key in it is explained (paste the path and the key list)
- [ ] log file + rotation verified: paste a run whose log file exists, a rotation actually triggered
      (small `log_rotate_bytes` to force it), and the kept file count bounded
- [ ] `pii health --json` exits 0 against a live daemon and **non-zero** with no daemon (paste both exits)
- [ ] `kill -9` + restart with the store intact: paste the before/after `pii health --json` counts and the
      restart command used
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate
      --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report names every file added/changed and the exact commands a user runs to deploy

## Guardrails
- Do NOT invent a static build you did not produce: a claim without `file`/`ldd` output is an overclaim.
- No new raw prints in `src/` beyond the file's budget; emit through `pii-observe`.
- Do not leave a daemon or container running at the end (kill what you start; `pgrep -af 'pii[ ]daemon'`
  must be empty in your report).
- Secrets (tokens, keys) never go into the config sample, the log file, or the health output.
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
**The static claim is now TRUE, and only where it is true.** The musl target installed, and the release
build produced a **4,596,744-byte static-pie binary**. `_docs/01-overview/001-pii-vision.md` was corrected:
the headline now reads "a single Rust binary (`pii`; statically linked when built for
`x86_64-unknown-linux-musl`)", with a status block stating that the DEFAULT build is dynamically linked —
and that `pii-web`, `pii-mcp` and `pii-evolve` were NOT measured. That last sentence is the honesty the
old claim lacked.

**Reproduced by the orchestrator (my runs):**
```
file target/.../musl/release/pii → ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped
ldd  → "statically linked"
pii daemon --help → rc=0, real flags listed;  --bogus → rc=2, "unknown flag"
health with NO daemon        → rc=2     health against a LIVE daemon → rc=0
   live: {"path":"…/data/store.redb","bytes":1056768,"runs":0}
kill -9 → restart same cmd → rc=0, store intact (same path, same bytes)
rotation (my own focused check): 5 daemon starts with --log-rotate-bytes 120 --log-keep-files 2
   → exactly pii.log 125B, pii.log.1 195B, pii.log.2 125B  (bounded at keep+1)
daemons left running → 0
systemd-analyze verify deploy/pii.service → rc=0
```
Gates: `quality-gate.sh` ALL PASS (incl. `PASS: lint-observability`) · `fresh-gate --worktree` PASS.

**Limits the worker stated, kept here:** the Dockerfile is UNTESTED (no Docker on this host); the systemd
unit passed `systemd-analyze verify` but was never run as a service; there is no RED-first evidence for the
CLI flag-parsing and config-key tests (code written before them). Scratch left: `/projects/_tmp/musl-target`
(313M, the musl target dir — reusable, left in place deliberately) and `/projects/_tmp/p104`.
Also recorded from the worker's report: the daemon does NOT create the store's parent directory (the unit
and the Dockerfile do), and a `kill -9` leaves no `daemon.stopped` line — a log ending on `daemon.started`
means the daemon died hard. Both are in the sample config's comments.
