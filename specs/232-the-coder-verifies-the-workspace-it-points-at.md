# Spec: the coder verifies the workspace it points at — a named criterion per workspace kind

## Overview
Measured 2026-10-20 on lnx (runs r212, r225, r257-r258, `_docs/05-operations/M0-2026-10-20.md`):

1. M0 PASS: the sandboxed daemon started `localhost/pii-agent:1` through the podman API socket —
   `coder.spawned{pid 3463854, runtime=socket, container b48d1fc9a4f4…}` + `coder.exited{same pid,
   rc 1, end_turn, criteria 0/0}`. The container path works.
2. The coder's PASS branch needs criteria (spec 221's event pair + a verdict), and the production
   criteria table names ONLY pii's own tests (`criteria.rs` PRODUCTION: coder_spawns_a_process,
   criteria_are_named_not_shell, grant_ceiling_refuses). A coder pointed at any other workspace
   (the demo ws `/tmp/pii-ws-3aQWp1`, measured) cannot name a check of ITS tree: adding
   `cargo:test:workspace/all` -> `cargo test` was attempted (83f6c1b) and reverted by hand (26ae945)
   because a fixed table entry that runs `cargo test` in ANY workdir the graph names is exactly the
   arbitrary-host-execution spec 215 removed — the graph author chooses the workdir.
3. The installed lnx graphs (coder-demo.toml, coder-podman.toml) still carry raw criterion strings
   (`cargo test -p pii-server --test coder_spawns_a_process`), which no table name resolves: the
   node refuses `criteria.unknown` before spawning (runs r212/r225 measured). Two real defects:
   the demo graphs are stale against spec 215, and no honest verifier exists for a non-pii tree.

Review 078 Q4 names the real fix: run the verdict in the image against a snapshot tree. Until that
lands, the coder cannot prove work on a workspace other than pii's own tree.

## Architecture
No new crate. Three seams, all existing:
- `crates/pii-server/src/node_kinds/nodes/coder.rs` — `resolve_criteria` gains the scope-checked
  workspace entry (it consults `ctx.trace()`'s scope root, which the node already receives); the
  refusal path is `refuse_criterion`, unchanged.
- `crates/pii-server/src/node_kinds/nodes/coder_runtime.rs` — `PodmanApi` inspects the created
  container for the digest before start; `Refusal::Runtime` carries it unchanged.
- `crates/pii-cli/src/cmd_trace.rs` — `fail_text` falls back to the terminal port's verdict text;
  `full_reason` and the display line are unchanged.
The named-criteria contract (spec 215) is untouched: the table stays const, the workspace entry is
one scope-checked row, and no shell interpolation appears anywhere.

## Requirements
R1. A criterion may be declared `cargo:test:workspace/all` ONLY on a node whose `workdir` equals the
    run's scope root (the workspace the daemon was started with): the graph author cannot point the
    check at an arbitrary tree by name; the daemon resolves `cargo test` against THAT workspace, and
    the argv0 scope stays `cargo` (already granted).
R2. The stale demo graphs on lnx are rewritten to named criteria (the pii ones keep pii's names;
    the demo ws graph uses R1's name) — graphs are data, the table stays compile-time fixed except
    for R1's scope-checked entry.
R3. `pii run trace --reason <node>` falls back to the node's terminal port verdict text when
    first_failure/refusal/failure are all empty (measured: r258 replied `no-failure` while the run
    get said `spawned=3463854 exited=1`).
R4. `PodmanApi` fills `coder.spawned.image_digest` (an inspect after create; empty string measured
    on r257/r258), so trace's image slot stops printing `-` for a socket-runtime run that worked.

## TDD Contract
- pii-server `tests/workspace_criterion_is_scope_checked.rs`: a graph naming
  `cargo:test:workspace/all` with workdir != scope root refuses `criteria.unknown`; equal workdir
  resolves to `cargo test` in that workdir (fake proc asserts argv+wd).
- pii-server `coder_runtime.rs` unit: `PodmanApi::run` sets image_digest from `/_images/{name}/json`
  after create; the existing fake-socket test extends without breaking.
- pii-cli `trace_reason_is_not_clipped.rs` gains a fixture whose node has empty failure fields but a
  terminal port verdict; `--reason` prints that text.

## Exit Criteria
- [ ] `cargo test -p pii-server --test workspace_criterion_is_scope_checked`
- [ ] `cargo test -p pii-server --test coder_starts_through_the_runtime_socket` (extended, still green)
- [ ] `cargo test -p pii-cli --test trace_reason_is_not_clipped`
- [ ] `! grep -rn 'cargo test -p pii-server' /run/user/1000/pii/graphs/` on lnx after the graphs are re-installed with named criteria

## Guardrails
- Do NOT re-add an unconstrained `cargo test` table entry (83f6c1b's shape): the scope check is the
  requirement, not an optional guard.
- Do NOT loosen spec 215's rules to make the demo pass: the demo graph gets fixed, not the table
  loosened.
- The demo ws run still may not be called agent-work-proof until its criterion runs agent-unmodifiable
  tests over a snapshot (078 Q4 stays open).
