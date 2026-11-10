# Spec: the digest cannot loop

## Overview
069, measured: graphs/discord-digest.toml carries a [[cron]] entry whose graph is NOT a timer node, so once the daemon can find its graphs it runs the digest every 17 ms - 58 runs per second. Fixing the graphs path without this turns a silent daemon into a flooding one.

**Project**: <repo>

## Requirements
Give the digest graph a timer node with cron = "0 9 * * *". AND make the daemon REFUSE at start any [[cron]] whose graph entry is not a timer, naming the entry - config.rs:457 documents that precondition and nothing enforces it.

## Exit Criteria
- [ ] cargo test -p pii-server --test daemon_graphs (a cron whose graph entry is not a timer must be refused at start, named)
- [ ] cargo test -p pii-server --test daemon_graphs

## Guardrails
- Do NOT weaken the gate, a budget or any existing control to make this pass.
