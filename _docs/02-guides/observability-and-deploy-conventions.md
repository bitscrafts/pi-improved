# Observability and deploy conventions (pii)

**Status**: current as of 2026-10-09 (specs 97-106). This is the same material that belongs in `AGENTS.md`;
it lives here because `AGENTS.md` is a protected file that needs explicit owner approval to edit. The
pointer paragraph for `AGENTS.md` is drafted in the handoff (`HANDOFF.md`), ready to paste on approval.

## Observability is enforced by the gate, not by convention

Emit through **`pii-observe`**, the contract crate (serde-only dependency floor: `serde` + `serde_json`).
`pii-core` re-exports it as `pii_core::contract`; the RUNTIME (multiplexer, sinks, registry) lives in the
engine crate, which implements the contract's traits.

- **`lint-observability` is a FAILING step in `quality-gate.sh`.** Every crate must depend on the contract
  and use it, **or** be listed in `xtask/observability-exemptions.txt` — a registry pinned to four
  sanctioned crates (`pii-abi`, `pii-proto`, `pii-uds`, `pii-observe`; pure types / the contract itself). A
  FIFTH entry is a red gate. A listed crate that DOES emit is a "dead exemption" red gate. The lint prints
  its registry size every run, so an exemption is a reviewable one-file diff.
- **Raw `eprintln!`/`println!`/`dbg!` are budgeted per file** in `xtask/observability-allowlist.txt`
  (`path  # reason  # N`). The budget may only shrink; adding a print to a listed file fails the gate. Per
  FILE, not `file:line`: the line-keyed version churned 16 diff lines around 142 entries for zero
  information on a single edit.
- **Every `pub const` in `crates/pii-observe/src/names.rs` must appear in `ALL`.** Assert on those symbols,
  never on string literals.
- **The lint fails closed**: an unreadable registry, a missing allowlist or a crate with no `src/` is a
  finding, never a silent skip. Its FAILURE path must be as trustworthy as its pass — an early version
  fabricated five findings (including `crate pii-observe does not depend on pii-observe`) when the registry
  was unusable.
- **The seam is compulsory where the language allows**: subsystems take their observer in the constructor,
  and the daemon's aggregator holds `&dyn Health` with a test pinning the registered set BY NAME. (No
  language can require "every crate implements X" — a trait binds a type, not a crate, in Rust or Java.)

## Health reports IDENTITY, not liveness

`pii health --json` reports: build (version, revision, target), config (path + sha256), stores (path, bytes,
**format_version**, entries per table, last write), mounts (the RESOLVED roots, mode, and the policy that
granted them), counts (runs, journal events, approvals pending, cron entries, memory docs), and per-subsystem
state. If a store has no format version, ADD one; a NEWER on-disk version is a loud refusal naming the file
and both versions. Never report a configured profile NAME in place of the resolved value.

## Operator surfaces must not lie

Measured defects, fixed in spec 106 — the pattern to test for on any new surface:
- `pii run ls` listed NOTHING by construction (`status.map_or_else(Vec::new, …)`), while
  `--status bogus` was swallowed into "no filter" and read exactly like "there are no runs" at exit 0.
  Now: unfiltered lists every run; an unknown status is REFUSED (rc 2) naming the accepted values.
- A suspended run waited for a human with `approve ls` empty and `approvals_pending: 0`, because the
  suspend handler read the journal BEFORE the write-behind flush and returned with no log. Anything a
  handler cannot handle must be REPORTED, never `let _ =`-ed away.
- `health` omitted a store the daemon opens (`timer.redb`): the reported set must equal the stores on disk,
  asserted by a test whose mutation control drops one and goes red.

## Trajectories (spec 103)

`PII_TRAJECTORY_DIR` (or `[core] trajectory_dir`), **default OFF**, append-only, one JSONL per run at
`<root>/trajectories/<run_id>.jsonl`, carrying `run`/`session` correlation and the engine's terminal
VERBATIM (`run.outcome`). Redaction lives in the SINK (`PII_TRAJECTORY_REDACT=a,b`), never at the call site.
A write failure is reported as an event and never aborts the run it observes. **A failed run must never
record success** — every measure and outcome field needs a negative case.

## Deploy and operate

- **Static build**: `cargo build --release --target x86_64-unknown-linux-musl -p pii-cli` →
  `static-pie linked` (verified). The DEFAULT build is dynamically linked — `001-pii-vision.md` states
  exactly that, and names `pii-web`/`pii-mcp`/`pii-evolve` as unmeasured.
- **Artefacts**: `deploy/Dockerfile` (present but UNTESTED — no Docker in this container),
  `deploy/pii.service`, `examples/pii.production.toml`.
- **Operator guide**: `_docs/05-operations/105-runbook.md` — start/stop, health, metrics, logs,
  backup/restore, upgrade, wedged daemon.
- **Commands**: `pii health [--json]` · `pii metrics` · `pii daemon --help` · `pii backup <dest> [--dir D]` ·
  `pii restore <src> [--dir D] [--force]`. Backup and restore are OFFLINE: they refuse while the daemon
  holds the store lock ("stop the daemon first"), and restore refuses a non-empty target without `--force`.
- **Operational facts**: the daemon does NOT create the store's parent directory (the unit and the
  Dockerfile do). A `kill -9` leaves no `daemon.stopped` line — a log ending on `daemon.started` means the
  daemon died hard. The bounded shutdown line is `pii: shutdown drain exceeded …` (spec 95, 5s bound).
- **Logs**: `--log-file` + `--log-rotate-bytes` + `--log-keep-files` (or the `[core]` equivalents); size
  based rotation with a bounded keep count.
