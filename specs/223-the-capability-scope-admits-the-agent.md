# Spec: a coder node's scope admits the agent binary it was told to spawn

## Overview
Measured 2026-10-20 on the host, daemon revision 0a78d6da0c20, three graphs, each with a coder node:

    coder-selffix (no image)                     → run failed in 1 ms · node "code" 0+0 · pid=- · 0 agent calls
    coder-podman  (container_image set)          → run failed in 2 ms · node "code" 0+1 · pid=- · image=- · 0 agent calls
    coder-demo    (fragment guard attached)      → refused by the guard, as designed

A coder node refuses before it reaches the spawn: no pid, no image digest, no argv. The graph grants `caps = ["spawn","proc"]`
and the node carries agent/model/image, so the refusal is happening in the capability scope, not in the agent selection.
Review 074 predicted exactly this (M4): the run scope pins `proc_argv0` and refuses an agent binary before any code runs.

## Requirements
1. A graph that declares `caps = ["spawn","proc"]` and a `coder` node gets a scope whose `proc_argv0` admits the agent binary
   resolved from `agent` (and, for an image run, `podman`), and nothing else.
2. The refusal is NAMED in the run's events: `worker.start.refused{agent, model, reason}` with the scope's own message, and
   `pii run trace` shows it on the node's line (`fail=<reason>`), not as an empty `pid=-`.
3. `image=-` must never appear for a node that declared `container_image`: the digest (or `missing`) is printed.
4. A refusal must be a terminal, distinct status (`refused`), never `failed`, so a scope defect is not mistaken for an agent
   failure (spec 221 already requires the distinction; this spec requires the evidence to be visible).

## Exit Criteria
- [ ] `cargo test -p pii-server --test coder_scope_admits_the_agent`
- [ ] `cargo test -p pii-server --test coder_refusal_names_the_scope_reason`
- [ ] on the host: `pii run start coder-podman` produces either `coder.spawned pid=<n>` or a trace line carrying the refusal reason
- [ ] `pii run trace <id>` shows `image=<digest8>` (or `missing`) for any node with `container_image`

## Guardrails
- Do NOT widen the scope to `["/bin/sh"]` or to the whole PATH: the fix is to admit the ONE binary the node names.
- Do NOT let a silent refusal look like a fast successful run (that is the 115 ms / 1 ms defect class of this night).
- Do NOT remove the capability model to make the demo pass.
