# Spec: 101 — adoption sweep, part 2: the clients, channels and the evolutionary loop

**Project**: /projects/pi-improved
**Phase**: P0/P1 observability — the second half of the adoption sweep (part 1 is spec 100, the
server-side subsystems). Enforcement hardening follows in spec 102, so the gate stays green here.

## Context
Same starting point as spec 100, for the crates that face the user rather than a subsystem: they hold the
largest concentration of raw prints in the repo (`pii-cli` had 103 of the 139 when the analysis was
written). Their observability is mostly *error and progress reporting* rather than subsystem metrics.

## Requirements
R1. **Convert these seven crates** — for each: add the `pii-observe` dependency, take or expose the seam,
    emit **at least one real event** through `pii_observe::names` consts at its boundary, DELETE its
    `// observability: none — …` placeholder line, and record in the report which raw prints each one
    replaced (the count removed from `xtask/observability-allowlist.txt` per crate):

    | crate | the observation to add (at minimum) |
    |---|---|
    | `pii-cli` | operator-facing failures as structured events (startup, daemon connect, graph errors) instead of bare `eprintln!`; the ndjson stream stays as-is (it is the documented output format) |
    | `pii-channels` | poll failures and send failures as events, plus a cursor-lag measure |
    | `pii-tui` | render/attach failures and reattach gaps as events |
    | `pii-web` | request failures and upstream (daemon) errors as events |
    | `pii-mcp` | tool-call failures as events |
    | `pii-evolve` | cycle start/finish and launch failures as events (this crate is the boss/cron launcher — its failures are currently only prints) |
    | `pii-http-fetch` | fetch outcomes and **allowlist denials** as events (it is a real fetcher with a `http:<host>` grant key, not a types-only crate) |
R2. **Leave exactly four sanctioned exemptions**, and make each reason truthful and short (they become a
    checked-in registry in spec 102): `pii-abi` (kernel ABI: traits/types only), `pii-proto` (wire
    types/consts only), `pii-uds` (path helpers only), `pii-observe` (the contract itself). Measured on
    HEAD after spec 100 (orchestrator): the 11 remaining exemptions are exactly these four plus the
    seven crates listed in R1 — so this spec ends with the sanctioned four only. Do not exempt
    anything else; if a crate in the list above genuinely cannot emit, say so in the report with the
    reason instead of exempting it.
R3. **Adequacy floor for each converted crate**: one passing test asserting a **named** event from it
    (`crate → test → name` table in the report, tests actually run and pasted).
R4. **Net print reduction**: report the allowlist total before and after (it must DROP; give both
    numbers). Do not add entries.
R5. `specs/` is read-only for you.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- lint-observability` exits 0 and its summary shows the exempt count is now exactly **4** (it was 11 after spec 100; paste the summary line)
- [ ] `[ "$(grep -rn '// observability: none' crates/ | grep -vE 'pii-abi|pii-proto|pii-uds|pii-observe' | wc -l)" = 0 ]`
- [ ] the print allowlist total dropped — both numbers pasted, and the per-crate deltas stated
- [ ] the adequacy table (`crate → test → name`) is in the report and each test passes when run (paste)
- [ ] `cargo test --workspace --lib --bins --tests` exits 0; `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0; `lint-files` clean
- [ ] `pii -p "reply with exactly: PONG" --model ollama:granite4.2:3b` still prints its ndjson stream unchanged (paste a line) — the CLI's documented output must not regress
- [ ] the report lists every file changed per crate

## Guardrails
- **Do not change the CLI's documented ndjson output** — replace bare prints, not the structured stream.
- Every converted crate needs its R3 test; an event nothing asserts is the defect class this repo keeps
  finding.
- Do NOT touch the exemption mechanism, the allowlist keying, or `quality-gate.sh` beyond what exists
  (spec 102 does that).
- Do NOT log secrets, tokens or env values.
- Do NOT edit any file under `specs/`.
- Scratch under `/projects/_tmp/`; delete your own exports (one is ~8G).
- Do NOT commit; leave the tree for the orchestrator.

## Result + orchestrator verification (2026-10-09)
**The target state is reached: `lint-observability` now reports `187 files checked, 96 print sites
allowlisted, 4 crates exempt, 0 findings`** — and the four carry truthful reasons:
`pii-abi` "kernel ABI: traits and types only, nothing to observe" · `pii-proto` "wire types and consts
only" · `pii-uds` "path helpers only" · `pii-observe` "this is the contract crate itself".
Raw prints dropped **140 → 96** (`pii-cli` 104 → 62; `cli_util` 4→1, `cmd_ctl` 15→13, `cmd_daemon` 16→6,
`cmd_oneshot` 11→4, `cmd_run` 5→2, `cmd_sandbox_exec` 6→3, `cmd_soak` 4→3, `cmd_sys` 5→4,
`cmd_trace` 15→12, `cmd_tui` 5→0, `main` 16→12). Seven crates gained named events + adequacy tests.
Both gates green.

**Verified by the orchestrator:**
- The lint summary above (4 exempt, 0 findings), and each of the four reasons read in full.
- Adequacy tests run: `pii-http-fetch --test observe_contract` (2 ok), `pii-channels --test
  observe_contract` (2 ok).
- **Mutation control (mine, at the EMIT site)**: renaming
  `&Event::new("fetch", Level::Warn, names::FETCH_ALLOWLIST_DENIED)` →
  `allowlist_denial_is_reported_by_name` FAILED with the full event dump showing the mutant kind.
  Restored, 0 mutant occurrences.
- **The CLI guardrail held**: `pii -p "reply with exactly: PONG" --model ollama:granite4.2:3b` still
  emits the documented stream (`node_started` → `node_finished` → `run_finished`, same keys and shape).

**A verification near-miss, recorded because it is the same class this repo keeps catching:** my FIRST
mutation attempt replaced a `names::FETCH_ALLOWLIST_DENIED` occurrence **inside a doc comment** (observe.rs
line 35) rather than the emit at line 55 — and the test stayed GREEN. Read carelessly, that would have
been filed as "the adequacy test has no teeth". The truth was the opposite: my mutation hit the wrong
line. Suspect the mutation first; a green mutant is a claim about the *edit* until the edit is confirmed
to be at the emit site.
