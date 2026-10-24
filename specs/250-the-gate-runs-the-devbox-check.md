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
