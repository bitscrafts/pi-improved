# Spec: 98 — `lint-observability`: make the seam and the ban on raw prints FAIL the gate

**Project**: /projects/pi-improved
**Phase**: P0/P1, observability — this is the ENFORCEMENT half of the user's directive
(*"we must enforce observability! via traits on every crate!"*). Design/analysis:
`_docs/06-research/053-observability-contract-and-enforcement-2026-10-09.md`.

## Context
Spec 97 landed the contract (`pii-observe`, serde-only). A rule that is not enforced is a rule that
gets broken — this repo's own history (the `lint-scratch` class, the spec-77 criterion that drove 19
files of debt) says a convention enforced by prose dies on the next cheap worker run. This spec adds
the check that fails the gate when a crate has no observability seam or when new raw prints appear.

## Requirements
R1. **New xtask subcommand `lint-observability`** (same shape as `lint-scratch`/`reachability-check`:
    prints findings as `path:line: message`, exits non-zero on any finding, prints a one-line summary
    with counts). Three rules:
    - **R1a — the dependency:** every crate directory in `crates/` must depend on `pii-observe`
      (a `pii-observe = { workspace = true }` line in its `Cargo.toml`), UNLESS it carries a named
      exemption (R1c).
    - **R1b — the seam:** every crate must contain at least one use of the contract in `src/`
      (`Observer`, `Observable`, `Event`, `Measure`, `Health`, or a `pii_observe::` path), UNLESS
      exempted (R1c).
    - **R1c — named exemptions:** a crate may declare `// observability: none — <reason>` (exactly one
      line, in `src/lib.rs` or `src/main.rs`). The reason must be non-empty; the message must name the
      crate. Exemptions are ALLOWED to be rare and are listed in the report.
    - **R1d — no new raw prints:** `eprintln!`, `println!`, `dbg!`, `print!` in `crates/*/src/**` are
      findings UNLESS the `file:line` is in the checked-in allowlist
      `crates/xtask/observability-allowlist.txt` (one `path:line  # reason` per line). Seed that file
      with the CURRENT sites (measure them; the analysis counted 139 — use the real number your run
      finds) so the lint lands GREEN, and note the count in the report. Deleting allowlist lines is the
      adoption sweep's job (spec 100); ADDING one requires a reason on the same line.
R1e — **registry completeness** (orchestrator-measured gap, spec 97's own finding): the contract's name
    registry (`pii-observe/src/names.rs`) guards only its `ALL` slice. Proven: an unregistered const with
    a duplicate value left all 9 tests green. The lint must therefore require **every `pub const` in
    `names.rs` to appear in `ALL`**, and that no name in `ALL` is defined outside that module — so a new
    name cannot be added and silently never enter the vocabulary the lint and dashboards read. Add a
    matching negative control: an unregistered const in an export must make the lint exit non-zero.
R2. **Wire it into `quality-gate.sh` as a FAILING step**, placed with the other xtask checks (after
    `lint-scratch`, before the summary), with the same `PASS:`/`FAIL:` output shape.
R3. **Teeth, proven in a `git archive` export** (do not mutate the repo tree):
    - re-introduce one raw `eprintln!` into a crate's `src/` → `lint-observability` exits non-zero and
      names that `path:line`;
    - remove a crate's `pii-observe` dependency (or its only contract use) in the export → exits
      non-zero naming that crate;
    - restore → exits 0 with 0 findings;
    - **R1e control**: add an unregistered `pub const` to `names.rs` in the export → exits non-zero
      naming it.
    Record the commands and their output verbatim in the report.
R4. **Unit tests for the lint itself** in `crates/xtask` (the repo's pattern): the allowlist parser
    (comment/reason handling), the exemption parser (a bare `// observability: none` with no reason is
    a finding), and the "no duplicates in the allowlist" invariant.
R5. Do NOT convert any call site in this spec. The sweep is spec 100; this spec's job is that the rule
    exists and bites. `specs/` stays read-only for you.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- lint-observability` exits 0 on the untouched tree and prints a summary line with the file/finding counts (paste it)
- [ ] `[ "$(grep -c 'lint-observability' quality-gate.sh)" -ge 1 ]` and `bash quality-gate.sh` exits 0 with a `PASS: lint-observability` line (paste it)
- [ ] the three export controls from R3 each show the expected exit code, verbatim
- [ ] `cargo test -p xtask` passes (including the new lint tests)
- [ ] `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report states: the number of allowlist entries seeded, the number of crates exempted (with their reasons), and the number of crates currently failing R1b (i.e. the work spec 100 must do)
- [ ] the report confirms `specs/` was not touched

## Guardrails
- The lint must FAIL CLOSED: an unreadable `Cargo.toml`, a missing allowlist file, or a crate with no
  `src/` is a finding, not a silent skip.
- Do NOT weaken the allowlist rule to make the lint pass: seeding it with the CURRENT sites is honest;
  adding entries for sites you create is not.
- Do NOT touch `lint-files`, `lint-scratch` or `reachability-check` behaviour, and do not reorder the
  existing gate steps other than inserting the new one.
- Do NOT edit any file under `specs/`.
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
Landed: `xtask/src/lint_observability.rs`, `xtask/observability-allowlist.txt` (140 entries), the
`lint-observability` gate step (`grep -c` = 4), and one `// observability: none — …` line added to the
19 crates' entry files.

**Verified by the orchestrator, not taken on report:**
- `lint-observability` on the tree → exit 0, with an honest summary:
  `174 files checked, 140 print sites allowlisted, 19 crates exempt, 0 findings`, and it LISTS each
  exemption with its reason.
- **Control A (raw print bites):** re-introducing `eprintln!` at `crates/pii-uds/src/lib.rs:1` →
  exit 1, `crates/pii-uds/src/lib.rs:1: raw print macro (use the pii-observe seam, or allowlist with a
  reason)`. Restored.
- **Control B (R1e — my own spec-97 finding, now closed):** appending an unregistered
  `pub const ROGUE_PROBE` to `names.rs` → exit 1, ```ROGUE_PROBE` is not listed in ALL``. Restored.

## HONEST CAVEAT — the enforcement is real, the ADOPTION is not yet
The lint lands green **only because all 19 crates carry a placeholder exemption** with the literal reason
*"pending the spec 100 adoption sweep (no seam yet)"*. `pii-core` is the only crate that passes on a real
use site. So today this spec enforces a *shell*: the rule exists, bites, and makes the debt **visible and
countable** (the lint prints the count) — but 19 crates are exempt from the rule the user asked to
enforce on every crate.

That is deliberate staging, and it is only acceptable because it cannot become permanent: spec 100 now
carries hard exit criteria that (a) the placeholder string appears **nowhere**, (b) the exempt set shrinks
to the sanctioned pure-type crates (`pii-abi`, `pii-proto`, `pii-uds`, `pii-observe`), and (c) every other
crate passes R1b on genuine `pii_observe::` use. Until spec 100 lands, this row must not be read as
"observability is enforced on every crate".

Process notes from the worker, recorded as-is: it ran `git stash` by mistake and popped it immediately
(all 23 paths restored); a stale xtask test binary in the shared target dir failed two xtask tests until
rebuilt. Also observed in the tree (pre-existing, untracked, 2026-09-18): a directory literally named
`crates/*` and `crates/_tmp/phase3/slice-e-it` — junk from a bad glob, which the lint has to skip; flagged
for cleanup, not deleted.
