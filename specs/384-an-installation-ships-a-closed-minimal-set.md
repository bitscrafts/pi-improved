# Spec: an installation ships a closed, minimal set of graphs

## Overview

The operator's requirement: *"the pii should ship with a set of graphs for a minimal working
environment"*.

Today the installer copies the directory. That is not a set: it is whatever happens to be in
`graphs/` at install time, including samples, one-off probes, and — measured 2026-11-04 — **two files
that cannot load at all** (`code-index.toml`, `intervention-report.toml`: `error: invalid: missing
'name'`). A host that installs the directory therefore ships a library whose own linter would refuse
part of it, and no operator can tell which of twenty-seven files is the working environment.

The set is not a matter of taste. It is the **call closure** of the flows that make a pii host useful:

```
night        -> sdd -> { tdd, docs }
queue        -> spec -> docs
self-improve -> queue -> spec -> docs
books        -> docs        (+ docs-stale, spec 383)
doc-code     -> docs
```

Everything in that closure is required; everything outside it is a sample.

**Project**: <repo>
**Phase**: installation


## The set, from derivation 047 (do not re-derive it)

```
system (11)   spec · method-judge · sdd · docs · queue · self-improve · night · books · digest-now ·
              code-index · preflight
user          everything the operator writes or copies, in ~/.local/pii/graphs/ — never written by an
              upgrade
direction     a user graph MAY call a system graph; a system graph must NOT call a user graph
```

**Three layers enforce the direction, and the first is structural:**

1. **Search path** — `load_host_with_graphs` already takes the dirs to search
   (`crates/pii-graph/src/fragment/weave.rs:112`). A host loaded from the system dir searches `[system]`
   only, so a system graph cannot resolve a user target whatever its text says. A user host searches
   `[system, user]`.
2. **No shadowing** — a user file named like a system graph is REFUSED at load, not preferred:
   shadowing `docs` would make every system graph call user code by name.
3. **Install-time and CI** — every manifest graph lints, every `call` target is in the manifest, every
   pin matches, before any file moves.

**Pins across an upgrade:** the installer keeps every replaced system revision under
`system/.by-sha256/<digest>.toml`; a `call` whose pin differs from the current file resolves there, so a
user graph keeps running the bytes it reviewed. `pii graph diff` and `pii graph repin` are the verbs that
move it forward. Rejected: refusing the upgrade (one forgotten file blocks every fix); re-resolving the
pin with a report (a run nobody reviewed); name+version (a label does not bind content).

**Samples:** `coder-demo`, `coder-podman`, `claude-probe`, `discord-triage` are examples to copy;
`coder-selffix` and `discord-digest` are obsolete in the repo (`discord-digest` survives on hosts
through the first-upgrade move); `digest-now` is system (F11).

## Requirements

- **R1 — the set is declared.** `graphs/minimal.txt` lists, one name per line, the graphs that
  constitute a working environment. Comments allowed; order is not significant.
- **R2 — the installer ships exactly that set.** `deploy/install-pii.sh` installs the graphs named in
  the manifest and no others. Samples stay in the repository and are installed only when named
  explicitly.
- **R3 — nothing that does not lint can ship.** The installer lints every manifest graph and refuses,
  naming the file, if one fails. The two currently broken files must therefore be fixed (spec 380) or
  dropped from the manifest — never shipped.
- **R4 — the set is closed.** Every `call` target of every shipped graph is itself shipped, and its
  pin matches the shipped file. A dangling pin on a fresh host is a run that cannot start; the
  installer refuses it rather than discovering it at 03:00.
- **R5 — the environment is provable.** After `install-pii.sh`, `pii graph list` names exactly the
  manifest's set, and a test installs into a scratch prefix, lists, and compares against the manifest.

## Exit Criteria

- [ ] `test -f graphs/minimal.txt`
- [ ] `grep -q 'minimal.txt' deploy/install-pii.sh`
- [ ] `grep -q 'lint' deploy/install-pii.sh`
- [ ] `cargo test -q -p pii-cli --test minimal_graph_set 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
- [ ] `grep -vE '^#|^$' graphs/minimal.txt | xargs -r -I{} ./target/debug/pii graph lint graphs/{}.toml`

## Guardrails

- Do NOT add a graph to the manifest to make the closure pass if the graph does not lint: fix it
  (380) or leave it out.
- Do NOT delete the samples from the repository. They are documentation of what the vocabulary can
  express; they are simply not the environment.
- Do NOT make the manifest a copy of `ls graphs/*.toml`. The point is that it is a decision.
- Do NOT install a graph whose pin does not match its file on disk.

## Files

- `graphs/minimal.txt`
- `deploy/install-pii.sh`
- `crates/pii-cli/tests/minimal_graph_set.rs`
- `graphs/README.md`
