# Spec: 99 — health must report IDENTITY: store, mounts, version, counts

**Project**: <repo>
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
- Scratch under `<scratch>/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
Landed: `pii-store/src/identity.rs` (`StoreInfo`, `info()`, `Health for Store`/`MemStore`),
`FORMAT_VERSION` + stamp-or-refuse in `open`, additive `pii-proto/src/sys.rs` fields, the daemon
aggregator, and `pii health [--json]`. 12 files; both gates green.

**Verified by the orchestrator from a LIVE daemon (the artefact used as an operator would)** — I started
a daemon in a scratch dir and ran the new command myself:

```
$ pii health --json
{"build":{"revision":"5c6fe70cd7ee-dirty","target":"x86_64-unknown-linux-gnu","version":"0.1.0"},
 "config":{"path":"~/.pii/pii.toml","sha256":"0b800423…"},
 "counts":{"approvals_pending":0,"cron_entries":0,"journal_events":0,"memory_docs":0,"runs":0},
 "mounts":[{"mode":"rw","name":"workspace","root":"<scratch>/health-orch-demo/ws",
            "policy":"sandbox=off: no filesystem confinement applied"}, …channels_scope, runtime_dir…],
 "sandbox_profile":"off","protocol":"pii.v2","rss_kb":18292,
 "stores":[{"name":"main","path":"…/store.redb","bytes":1056768,"format_version":1,
            "last_write_ms":1791564480077,"entries":{"graph_body":0,"run":0,"run_journal":0,…}},
           {"name":"memory","path":"…/memory.redb","bytes":1056768,"format_version":null,…}],
 "subsystems":[{"name":"store","state":"ok","identity":{…}}, {"name":"memory",…}, {"name":"net",…}]}
```

That satisfies the directive's four requirements — store, mounts, version, counts — and note the mount
policy string states the sandbox absence honestly (`sandbox=off: no filesystem confinement applied`)
rather than printing a profile name.

**The directive's decisive test, verified by the orchestrator with my own mutation control:**
`two_daemons_differ_in_identity_fields` passes on HEAD; blanking the store path at its construction site
(`crates/pii-store/src/identity.rs`, `path: self.path.clone()` → `PathBuf::new()`) turns it RED —
```
panicked at health_identity.rs:111:5: assertion `left != right` failed:
different stores must report different paths
```
Restored and re-verified green (0 mutant occurrences left).

## Findings / residual (honest)
1. **The memory store reports `format_version: null`** while the main store reports `1` — the stamp-or-refuse
   path covers `Store` but not every redb file the daemon opens. A store whose version is unknown cannot be
   refused on upgrade, so P1's migration story (spec 102) must stamp them all.
2. **The aggregator registers 3 subsystems today** (store, memory, net); worker/channels/graph are not yet
   contributors. Spec 100's R9 pins the registered set by name, which is what makes that visible.
3. `pii health` exits 2 with `error: no daemon answered …; is \`pii daemon\` running?` when nothing listens —
   the honest failure mode, verified.
