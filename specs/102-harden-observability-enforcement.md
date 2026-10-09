# Spec: 102 — harden the enforcement: a bounded exemption registry and a print budget

**Project**: /projects/pi-improved
**Phase**: P0/P1 observability — the LAST step of the enforcement lane, landing only after both adoption
sweeps (specs 100, 101) so the gate stays green throughout.
**Why this exists** (orchestrator findings, 2026-10-09): as landed in spec 98, the exemption is a comment
*inside the crate's own source* (`const EXEMPT_MARK: &str = "// observability: none"`), so any run can
exempt any crate by typing one line — spec 98 went green exactly that way for 19 crates. And the print
allowlist is keyed `file:line`, so it churns on every edit: spec 99 shifted 12 lines in one file and the
allowlist diff was 16 lines for 142 entries before and after — pure churn, zero information.

## Requirements
R1. **Move the exemption set out of the crate sources** into a checked-in registry
    `xtask/observability-exemptions.txt`, one `<crate>  # <reason>` per line. The lint reads THAT file,
    not the sources. Remove every `// observability: none` line from `crates/`.
R2. **Pin the sanctioned set**: only `pii-abi`, `pii-proto`, `pii-uds`, `pii-observe` may appear, each
    with a non-empty reason. A fifth entry (or a missing reason, or an unknown crate name) is a FINDING.
    The lint must report the registry size in its summary line, so an added exemption is a visible,
    reviewable one-file diff.
R3. **Fail a dead exemption**: a crate listed in the registry that ALSO carries a real seam (a
    `pii_observe` use site) is a finding — exemptions may not linger after adoption.
R4. **Re-key the print allowlist as a per-file BUDGET** (R6b of the old plan): each entry becomes
    `path  # reason  # N` where N is how many raw prints that file may still contain; the lint fails when
    a file contains MORE than its budget. Consequences to verify: an edit that shifts lines inside a
    listed file no longer changes the allowlist at all; adding one print to a listed file fails.
    Do NOT switch to an inline `// obs-allow` marker at the print site — that would reintroduce the
    self-granted exemption R1/R2 exist to remove.
R5. **Keep the gate failing-closed** on every input it reads: a missing/unreadable registry file, an
    unparsable Cargo.toml, a crate with no `src/`, or a budget line with no reason is a FINDING.
R6. `specs/` is read-only for you.

## Exit Criteria
- [ ] `cargo run -q -p xtask -- lint-observability` exits 0 with a summary naming the registry size (4) and the per-file budget total; paste it
- [ ] `[ "$(grep -rn '// observability: none' crates/ | wc -l)" = 0 ]` and `[ "$(grep -c '#' xtask/observability-exemptions.txt)" = 4 ]`
- [ ] **control 1 (unsanctioned exemption)**: adding a 5th crate to the registry in an export → exit non-zero naming it (paste)
- [ ] **control 2 (dead exemption)**: giving a listed crate a real seam in an export → exit non-zero as a dead exemption (paste)
- [ ] **control 3 (budget)**: adding one extra print to a file already at its budget → exit non-zero (paste), and restoring → exit 0
- [ ] **control 4 (no churn)**: shift ≥10 lines above a listed print (comment insertion) and show the allowlist needs NO edit and the lint still exits 0 (paste)
- [ ] **control 5 (fail-closed)**: an unreadable/absent registry file → exit non-zero (paste)
- [ ] `bash quality-gate.sh` exits 0 with `PASS: lint-observability`; `cargo run -q -p xtask -- fresh-gate --worktree` prints `fresh-gate: PASS`
- [ ] `cargo test -p xtask` passes (unit tests for the registry parser, the budget parser, and the dead-exemption rule)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` exit 0
- [ ] the report states the allowlist total before/after the re-key and confirms `specs/` was not touched

## Guardrails
- This spec makes the lint STRICTER; it must land with the tree already compliant (specs 100/101 landed),
  so any red is a real finding, not a staging artifact.
- Do NOT relax any existing rule to make a control pass; the controls above are the point.
- Do NOT touch `lint-files`, `lint-scratch` or `reachability-check` behaviour.
- Do NOT edit files under `specs/`.
- Scratch under `/projects/_tmp/` only (exports are small here; still delete your own).
- Do NOT commit; leave the tree for the orchestrator.

## Result (2026-10-09) — landed, then hardened twice by orchestrator controls
**Spec 102 as delivered (12 turns, $0.36):** `xtask/observability-exemptions.txt` holds the four
sanctioned crates with reasons; every `// observability: none` line is gone from `crates/`; the print
allowlist became a per-file budget (96 sites over 20 files); the dead-exemption rule is keyed on a
`pii_observe::` path (not on contract words — those appear in the contract crate itself, which would have
flagged it); fail-closed on unreadable inputs; 75 unit tests.

**Orchestrator-verified controls (mine, all pasted):**
- **No churn (R4)**: inserting 12 comment lines above the listed prints left the allowlist byte-identical
  (`sha256` `5a32e552f014` before and after) and the lint at 0 findings. The churn defect is closed.
- **Budget (R4)**: one extra `eprintln!` in `cmd_ctl.rs` → `1 findings`, exit 1; restored → exit 0.

**TWO DEFECTS FOUND BY MY OWN CONTROLS, FIXED HERE (orchestrator):** the failure path fabricated
findings. With a fifth registry entry (or with the registry file absent) the lint reported the true
finding **plus five false ones** — `crate pii-abi/pii-proto/pii-uds does not depend on pii-observe`,
`pii-uds makes no use of the contract`, and the absurd
**`crate pii-observe does not depend on pii-observe`** (a crate required to depend on itself). Two fixes:
1. `exempt_reason()` — the sanctioned set is now honoured from the const, independent of whether the
   registry file is readable or valid, so an unusable registry yields exactly ONE finding and no cascade.
2. the contract crate is never subject to the self-dependency rule (`CONTRACT_CRATE`).
Re-run after the fix:
```
registry absent  → 1 finding: "xtask/observability-exemptions.txt:1: registry unreadable: No such file…"
fifth entry      → 1 finding: "…:5: `pii-core` is not in the sanctioned exemption set (pii-abi, …)"
healthy tree     → 0 findings, registry size 4 (4 crates exempt)
```
Two unit tests pin both behaviours (`sanctioned_set_is_honoured_without_a_usable_registry`,
`contract_crate_is_never_required_to_depend_on_itself`).

**Process note (mine):** my first edit of the lint silently did not persist (I checked only the tool's
report, not the file), and my first version of the new tests tripped the workspace's own lints
(`expect_used` is DENIED here, and `std::fs::` was an unnecessary qualification) — the gate caught both
(`FAIL: clippy`). Fixed and re-verified. Reading back the artefact, not the tool's success message, is the
lesson; it is the same one this repo keeps paying for.
