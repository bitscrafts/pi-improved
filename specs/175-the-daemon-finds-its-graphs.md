# Spec: the daemon finds its graphs

## Overview
Measured 2026-10-17 on the DEPLOYED daemon (068, opus, C-run): revision 0e34719dce0d is alive under systemd with
runs: 0, 0 journal events, 0 graph bodies, and a digest that has never fired. Cause: the daemon loads graphs from
runtime_dir.join('graphs') = /run/user/0/pii/graphs, AND THAT PATH DOES NOT EXIST; the graphs are installed in
/root/.local/state/pii/graphs. Nothing the product is configured to do in production has ever run.

**Project**: /projects/pi-improved

## Requirements
The daemon must load the graphs it is configured to load, or refuse loudly at start-up naming the directory it looked in
and the one the installer used. A silent empty graph set is the defect.

## Exit Criteria
- [ ] cargo test -p pii-server --test daemon_graphs
- [ ] grep -q graphs crates/pii-cli/src/cmd_daemon.rs

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
