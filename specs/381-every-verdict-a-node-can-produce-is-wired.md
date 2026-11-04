# Spec: every verdict a node can produce is wired

## Overview

Three defects, one shape: a node can leave by a port the graph does not wire, so the verdict is
produced and then dropped. The run stops without recording which verdict it reached, and every
downstream reader — the queue row, the night report, the operator — sees a run that simply ended.

```
tdd    pii graph lint graphs/tdd.toml     rc=3   V9: multi-arm joins with 1 inbound data edge
queue  select declares ok|blocked|empty|running; queue.toml wires the first three
night  sdd can return refused|unverified; night.toml wires neither
```

The `night` case is the row-364 defect one layer up: 364 taught a `call` to return a callee's
refusal, and the graph that calls it still does not say what to do with one.

Found by documentation work, 2026-11-04 (`manual-audit-20261104.md`, F-1, F-2, F-3).

**Project**: /projects/pi-improved
**Phase**: graph library integrity

## Requirements

- **R1** `graphs/tdd.toml` lints clean: `pii graph lint graphs/tdd.toml` returns 0, with no V9.
- **R2** `graphs/queue.toml` wires `select.running` to a recorded outcome — a row held by another
  run is a verdict, not a silence.
- **R3** `graphs/night.toml` wires both refusal ports of its `sdd` call to a report that names the
  refusal, so a night that refused says so.
- **R4** Each wiring has a test that would fail if the edge were removed (not merely a lint pass).

## Exit Criteria

- [ ] `./target/debug/pii graph lint graphs/tdd.toml`
- [ ] `grep -q 'running' graphs/queue.toml`
- [ ] `grep -qE 'refused|unverified' graphs/night.toml`
- [ ] `cargo test -q -p pii-server --test queue_calls_spec 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `cargo test -q -p pii-server --test night_calls_sdd 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`

## Guardrails

- Do NOT silence V9 by disabling the lint.
- Do NOT invent a new node kind for a held row: `select` already declares the port.
- Do NOT let `night` swallow a refusal into `ok`.
