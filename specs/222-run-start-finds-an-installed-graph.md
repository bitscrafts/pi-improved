# Spec: `pii run start <name>` finds a graph that `pii graph list` already shows

## Overview
Measured 2026-10-20 on the host, daemon started fresh with ~/.local/state/pii/graphs populated:

    pii graph list        → coder-demo · ae84ecf558cc · nodes 5 · manual · armed 0 · installed-only
                            coder-selffix · f77d7d8c534c · nodes 3 · manual · armed 0 · installed-only
    pii graph show coder-demo → the file's content
    pii run start coder-demo  → error: 102: not found: graph 'coder-demo'
    pii run start coder-selffix → error: 102: not found: graph 'coder-selffix'

The reader that lists graphs and the reader that starts them disagree about where a graph lives. On the container daemon
the same command worked, so the difference is a lookup path — and a user cannot start what the tool says exists.

## Requirements
1. ONE resolution function for both: `graph list`, `graph show`, `graph explain`, `graph check` and `run start` must resolve a
   graph name through the same code path, or the disagreement is inevitable.
2. When a name does not resolve, the error names every directory searched and the number of graphs found there:
   `not found: graph 'x' — searched <dir> (0) <dir> (2)`.
3. A directory-listed graph is startable without any extra registration step; if registration IS required, `graph list` must
   say so per row (`installed-only` already hints at it — it must become an instruction).
4. The container and host daemons behave identically for the same directory layout.

## Exit Criteria
- [ ] `cargo test -p pii-cli --test run_start_resolves_like_graph_list`
- [ ] `cargo test -p pii-server --test unresolved_graph_names_the_directories`
- [ ] on the host: `pii graph list` shows a graph and `pii run start <that name>` returns a run id

## Guardrails
- Do NOT add a second registry: the directory IS the registry (spec 213's list/explain reads it).
- Do NOT make `run start` fall back to "the first graph found": a wrong-graph run is worse than a refusal.
