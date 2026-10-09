# Spec: 99 — health must report IDENTITY: store, mounts, version, counts

**Project**: /projects/pi-improved
**Phase**: P0/P1, observability — this is the user's standing directive, verbatim:
*"the health endpoint must report store, mounts, version, and counts to make divergences visible
immediately"* (2026-09-20). Today `sys.status` reports liveness and counters but **no store identity,
no mounts, no build/config identity** — two daemons serving different stores look identical.

## Context
`crates/pii-server/src/daemon/status.rs` builds `StatusResult` (uptime, runs_active, sessions,
sandbox_profile *string*, protocol version, rss_kb, workers, listeners, channels, memory docs). The
missing identity is enumerated in `_docs/06-research/053-…md` §1.1, and the contract it plugs into is
`pii_observe::Health`/`SubsystemHealth` (spec 97).

## Requirements
R1. **Store identity — and MEASURE it first.** Determine what the redb stores can report today:
    path, file size, table names/counts, last write. If there is **no schema/format version stored**,
    add one (a `META` entry like `store.format_version`, written on first open, and a refusal with a
    clear message when the on-disk version is NEWER than the binary understands — that refusal is also
    what P1's upgrade story needs). Report what you found, not what you assumed.
R2. **Mounts/scopes in force**: report the RESOLVED roots the process actually allows — workspace root,
    `channels_scope`, runtime dir — each with its mode (rw/ro) and the policy that granted it. A
    profile *name* is not identity; the resolved path is.
R3. **Version identity**: binary version, build identity (git sha or build id — use what the build
    system can provide; `env!("CARGO_PKG_VERSION")` plus the best available revision), target triple,
    and the config file path **plus a hash of its contents**.
R4. **Counts**: runs total, journal events, approvals pending, cron entries, memory docs (keeping the
    existing `memory.docs`), store table entry counts where cheap.
R5. **Payload + compatibility**: extend `StatusResult` (in `pii-proto`, so the wire type is versioned
    with everything else) with the new fields; existing fields keep their names and meaning. Aggregation
    goes through `pii_observe::Health` implementations where a subsystem owns the data (store, net,
    worker, channels, memory), so the daemon does not hardcode every subsystem's fields.
R6. **The operator surface**: `pii health [--json]` in the CLI — connects over the existing UDS control
    plane and prints the status payload (human-readable by default, `--json` verbatim). Exit non-zero
    with a clear message when no daemon answers.
R7. **THE decisive test (the directive's own test)**: a test that builds TWO daemons with DIFFERENT
    store paths (and different scopes) and asserts the two payloads **differ in identity** — and that
    the difference is in the specific fields (store path, scope roots), not merely in uptime. Then the
    mutation control: with the store-path field omitted, that test must FAIL. Record both.

## Exit Criteria
- [ ] `pii health --json` against a running daemon prints a payload containing, at minimum: `stores[].path`, `stores[].bytes`, `stores[].format_version`, `mounts[].root`, `build.version`, `config.sha256`, `counts.*` (paste the real output, with paths you can vouch for)
- [ ] the two-store divergence test exists and PASSES; with the store-path identity field removed it FAILS (paste both outputs)
- [ ] `sys.status` keeps every pre-existing field name (a test asserts the old keys are still present)
- [ ] the store format-version refusal: opening a store whose recorded version is newer than the binary understands fails with a clear message (test, with the message pasted)
- [ ] `cargo test --workspace --lib --bins --tests` exits 0; `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0; `lint-files` reports no file over the warn threshold
- [ ] the report lists every file changed and states which parts of the identity could NOT be obtained (with the reason) rather than inventing values

## Guardrails
- **Report the resolved value, never a name.** A profile string or a configured-but-not-applied path is
  a finding, not a health field.
- Do NOT invent a version number that is not actually persisted — if the store has no format version,
  the work is to ADD one, not to print a constant.
- Do NOT break the wire format for existing clients: additive fields only, and `pii-proto`'s round-trip
  tests must still pass.
- Do NOT leak secrets into the payload: report the config **hash**, never its contents; never report env
  values, tokens, or provider keys.
- Do NOT edit files under `specs/`.
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.
