# Spec 250: the quality gate runs the devbox module's own check

## Overview
**Project**: /projects/pi-improved
**Phase**: production — the devbox is where specs are now implemented (user directive 2026-10-24)

`modules/devbox/check.sh` verifies, with no build, no container and no key read, that the devbox module says what it claims: the base is
the official slim rust image pinned to the same version as `rust-toolchain.toml`, no credential value is in the Containerfile or the compose
file, the project is mounted at its own path, Python is uv's only, claude's sessions persist, and so on. Each predicate carries a control that
must fail on a planted violation (25+ rows). Today nothing runs it: a change to the Containerfile that breaks one of those claims is found
only when someone happens to run the script by hand. The spec-implementing path now depends on this module, so its claims belong to the gate.

The gate's steps live in ONE file, `gate-steps.txt` (spec 146): `<label>|<shell command>`, one per line. This spec adds one line.

## Requirements
### Functional
- **R1.** `gate-steps.txt` gains exactly one step: `devbox-check|bash modules/devbox/check.sh`, placed after the `doc-drift-check` line, preceded by one
  comment line saying what it protects (`# Spec 250: the devbox module's claims (image pin, no secret, project path, uv-only, ...) hold; offline, pure text.`).
- **R2.** No other line of `gate-steps.txt` changes, and `gate-steps-quick.txt` is NOT changed (the quick list is for routine items; this is a commit-boundary check).
- **R3.** `bash modules/devbox/check.sh` exits 0 on the repo as it is.

### Non-Functional
- Offline and deterministic: the step must not need podman, a network, or any credential.

## Architecture
```
quality-gate.sh --reads--> gate-steps.txt --step--> bash modules/devbox/check.sh  (25+ PASS/FAIL rows, exit 1 on any FAIL)
```

## TDD Contract
No new Rust. The proof is two shell facts and one control, all in the Exit Criteria below:

| Check | Expected |
|---|---|
| the step line exists, once, with the exact command | `grep -c` is 1 |
| `check.sh` passes on the repo | exit 0 |
| CONTROL: on a copy of the module whose Containerfile base is broken (`FROM scratch`), `check.sh` exits non-zero | it does: a step that cannot fail certifies nothing |

## Exit Criteria
- [ ] `test "$(grep -c '^devbox-check|bash modules/devbox/check.sh$' gate-steps.txt)" = 1`
- [ ] `bash modules/devbox/check.sh > /dev/null`
- [ ] `t=$(mktemp -d) && mkdir -p $t/modules/devbox && cp modules/devbox/* $t/modules/devbox/ && cp rust-toolchain.toml $t/ && sed -i 's/^FROM .*/FROM scratch/' $t/modules/devbox/Containerfile && ! bash $t/modules/devbox/check.sh > /dev/null 2>&1`
- [ ] `test "$(git diff --numstat -- gate-steps.txt | cut -f1)" = 2`
- [ ] `git diff --quiet -- gate-steps-quick.txt`

## Guardrails
- Do NOT edit `modules/devbox/check.sh` or any other file under `modules/`; only `gate-steps.txt`.
- Do NOT reorder, reword or remove any existing step.
- Do NOT commit.

## Closure

Closed by the orchestrator 2026-10-24T06:17Z, verified **independently of the cycle's own report** on the tree the item
left (HEAD `9a5bee2`, the commit that landed the row is `7999bb5`).

| criterion | reading |
|---|---|
| the step line exists exactly once | `grep -c '^devbox-check\|bash modules/devbox/check.sh$' gate-steps.txt` = **1**, rc 0 |
| `check.sh` passes on the repo | rc **0**; the login rows this landing touched read `PASS claude's login is not a single-file bind mount`, `PASS control: the single-file-login predicate catches a planted single-file mount` |
| CONTROL: a planted broken base fails | a copy of the module with `FROM scratch` → `check.sh` rc **1**; the control is not vacuous |
| the commit is two added lines and no more | `git show --numstat --format= 7999bb5 -- gate-steps.txt` = `2 0` |
| the quick list is untouched | `git show --numstat --format= 7999bb5 -- gate-steps-quick.txt` = **0 lines** |

Criteria 4 and 5 are written against `git diff`, which is the right shape for the uncommitted tree the worker leaves;
after the commit `git diff` is empty (0, not 2), so the honest post-commit reading is the commit's own numstat, quoted
above. The criterion was not weakened — it moved from "the diff adds two lines" to "the commit adds two lines", the
same fact at the only place it still exists.

**Mutation, by the orchestrator.** The item's claim is *the gate runs this check*, not *the line exists*. So the
mutation broke the command inside the one live `gate-steps.txt` line
(`bash modules/devbox/check.sh` → `bash modules/devbox/check-DOES-NOT-EXIST.sh`) and drove the **real** gate with a
step list extracted from that live file (`grep '^devbox-check|' gate-steps.txt > steps.txt`,
`GATE_STEPS=steps.txt bash quality-gate.sh`) — not a hand-written list, and not `check.sh` run by hand:

- mutated: `quality-gate: FAILED`, rc **1**, log line `FAIL: devbox-check`;
- restored from a copy (plain `cp`, then `touch` — never `cp -p`, whose old mtime makes cargo reuse a stale artefact);
  `sha256sum` before `699a63e0244ffb0759922187fe3aacf32ba9d2831c59e4cb7ec0a2b34e3842db`, after the **same**;
- restored: `quality-gate: ALL PASS`, rc **0**, `PASS: devbox-check`.

Leftovers: the control and mutation scratch live under `/projects/_tmp/verify250-*`, `/projects/_tmp/gate-250-*.log`,
outside the repo. Nothing under `modules/` was edited; the only tracked file this item changed is `gate-steps.txt`.
