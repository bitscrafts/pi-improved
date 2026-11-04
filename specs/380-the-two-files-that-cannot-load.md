# Spec: the two library files that cannot load at all

## Overview

Writing the graph library chapter of the user manual meant opening every file under `graphs/` and
stating what it does. Two of them cannot be opened by the loader at all, so the chapter could not be
written honestly and the audit stopped there.

Both put `name` inside a `[graph]` table and express edges as `depends_on` node fields. The loader
reads a top-level `name`, an `entry`, and `[[edge]]` triples, so both are refused before anything is
parsed.

```
$ pii graph lint graphs/code-index.toml            error: invalid: missing 'name'   rc=2
$ pii graph lint graphs/intervention-report.toml   error: invalid: missing 'name'   rc=2
```

Found by documentation work, 2026-11-04 (`_docs/06-research/manual-audit-20261104.md`, F-4).

**Project**: /projects/pi-improved
**Phase**: graph library integrity

## Requirements

- **R1** Both files load: `pii graph lint` returns 0 for each.
- **R2** The fix is in the files, not in the loader: the loader's shape is the contract every other
  graph already satisfies, and loosening it would make the other 25 files ambiguous.
- **R3** The intent of each file is preserved — `code-index` indexes the code, `intervention-report`
  reports an intervention. A file that loads by being rewritten into a different graph is not a fix.
- **R4** A test loads every file under `graphs/` and fails naming the file that does not load, so the
  whole directory is covered rather than these two.


## Measured reality (2026-11-04, review 046)

Five files fail `pii graph lint`, in two different ways — and they are two different defects:

```
code-index.toml            rc 2   missing 'name'   (name nested in [graph], edges as depends_on)
intervention-report.toml   rc 2   missing 'name'   (same shape)
tdd.toml                   rc 3   V9  multi-arm join with 1 inbound data edge  (`join = "any"`)
sdd.toml                   rc 3   V9  same
night.toml                 rc 3   V9  same
```

`validate` is only consulted on the lint paths (`engine/dispatch.rs:405`,
`cmd_graph_installed.rs:274`), so the three V9 files may still start at runtime (**unverified**). So the
first task is to decide which side is wrong: a graph whose join is legal at runtime is a lint defect,
and a graph the engine would mis-join is a graph defect. Record the answer before changing either.

`night`, `sdd` and `tdd` are also among the 11 system graphs, so this row blocks the environment.

## Exit Criteria

- [ ] `./target/debug/pii graph lint graphs/code-index.toml`
- [ ] `./target/debug/pii graph lint graphs/intervention-report.toml`
- [ ] `cargo test -q -p pii-cli --test every_graph_loads 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `./target/debug/pii graph lint graphs/tdd.toml`
- [ ] `./target/debug/pii graph lint graphs/sdd.toml`
- [ ] `./target/debug/pii graph lint graphs/night.toml`
- [ ] `./target/debug/pii graph list 2>/dev/null | grep -q code-index`

## Guardrails notes (measured while implementing)

Turning V9 into a warning made `crates/pii-server/tests/acked_run_survives_kill.rs` fail: its control
borrowed the one-armed-`any` refusal as the way to obtain a *refused start*. The control now plants a
graph that fails V1 (an edge leaving a port the node does not have), which is a refusal no later rule
change can turn into a warning. The test's subject is unchanged: a refused start still hands back an
id and journals `Failed` with its reason.

## Guardrails

- Do NOT relax a lint rule to make a file pass. If V9 is wrong, fix the rule **and** prove the
  engine's real behaviour with a test that would fail if the rule were right — a rule changed on the
  strength of a belief is how a check becomes a false green.
- Do NOT rewrite `tdd`, `sdd` or `night` to satisfy a lint whose verdict the engine does not share.
  Establish first whether a `join = "any"` node with one inbound data edge runs correctly; the answer
  decides which side changes.
- Do NOT relax the loader to accept `[graph]`-nested `name` or `depends_on` edges.
- Do NOT delete either file to make the directory lint clean.

## Files

- `graphs/code-index.toml`
- `graphs/intervention-report.toml`
- `graphs/tdd.toml`
- `graphs/sdd.toml`
- `graphs/night.toml`
- `crates/pii-graph/src/validate.rs`
- `crates/pii-cli/tests/every_graph_loads.rs`
- `crates/pii-graph/tests/one_armed_any_join.rs`
- `crates/pii-server/tests/acked_run_survives_kill.rs`
