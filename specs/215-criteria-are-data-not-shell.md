# Spec: criteria are named, not shell; and the coder gets no shell

## Overview
Review 076 premise P3 (2026-10-19): coder `criteria` run ON THE HOST as `/bin/sh -c <string>` in the graph. So 209's "pii
installs nothing" is true of the coder node and false of its verifier — a criterion string is arbitrary host execution, and it
is the highest-risk, smallest-change item in the review's order (§4.3).

**Project**: /projects/pi-improved

## Requirements
1. A coder node's criteria are NAMED checks: `criteria = ["cargo:test:pii-server/coder_spawns_a_process", …]` resolved by the
   daemon to a fixed command table, not interpolated shell.
2. No `sh -c` anywhere in the coder/criteria path. An unknown criterion name is a REFUSAL (`criteria.unknown`), not a fallback
   to shell.
3. A grant ceiling in daemon config: the maximum capability set any node may request; a graph asking beyond it is refused at
   load (`graph.grant.ceiling`).
4. The refusal names the criterion, the node and the ceiling — so a worker's report cannot hide it.

## Exit Criteria
- [ ] `cargo test -p pii-server --test criteria_are_named_not_shell`
- [ ] `cargo test -p pii-server --test grant_ceiling_refuses`
- [ ] `grep -rn 'sh -c' crates/pii-server/src/node_kinds/nodes/coder.rs` is empty

## Guardrails
- Do NOT keep a shell escape hatch behind a flag: the flag becomes the new default.
- Do NOT widen the ceiling to make an existing graph load.
