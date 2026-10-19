# Spec: the code index is a pii verb, not a companion script

## Overview
2026-10-19, user challenge: "how is your intention when deploy the pii? you are building python scripts companion?" — correct and it is a drift I introduced twice today. `scripts/pii-index-code.py` and `scripts/pii-ingest.py` put capability OUTSIDE the product: they cannot ship in the binary, they are invisible to `pii --help`, they break the "everything is a graph" line, and on the lnx host they depend on a python that the daemon does not own. Meanwhile the missing half of graphify — `pii memory link` — is a three-line argument parser in `crates/pii-cli/src/cmd_ctl.rs` and a call to an RPC the server already exposes. A capability that exists as a script is a capability pii does not have.

**Project**: /projects/pi-improved

## Requirements
### Functional
- `pii index code [--root <dir>] [--max N]` — a CLI verb that walks `crates/*/src`, creates entities through the memory subsystem's own API (not through another process), and prints the counts it wrote.
- `pii memory link <from-id> <to-id> --rel <name>` — the argument parser that is missing today; it must accept entity ids, validate provenance, and return the created relationship id.
- `pii index code` creates the two relationship families: `module contains <symbol>` and `crate depends_on <crate>` (the latter from each Cargo.toml).
- `pii memory search` reports `graph` among the lanes once relationships exist, for a query naming an indexed symbol.
- The scheduled form is `graphs/code-index.toml` calling the VERB (a node whose work is `pii index code`), not a shell that calls python.

### Non-Functional
- No new runtime dependency: the indexer uses the same crates the daemon already links (std fs, the store, the memory API).
- Idempotent: re-indexing the same tree must not multiply entities or relationships (key on kind+name, provenance refreshed).
- `--max` bounds a pass; provenance paths longer than the store limit are truncated, not fatal.

## Exit Criteria
- [ ] `cargo run -q -p pii-cli -- index code --max 200` exits 0 and prints `entities=<n> relationships=<m>` with m>0
- [ ] `cargo test -p pii-cli --test memory_link` passes (a link round-trips: created, listed by `neighbours`, id returned)
- [ ] `pii memory tiers` reports `relationships` > 0 after an index pass
- [ ] `pii memory search "journal::append"` returns `"graph"` in at least one hit's `lanes`
- [ ] `grep -rc 'pii-index-code.py' scripts deploy graphs` is zero (no caller remains)

## Guardrails
- Do NOT add a python or shell dependency to the deployed product: the delivered artefact is the Rust binary.
- Do NOT index `_tmp/`, `target/` or vendored trees.
- Do NOT fabricate edges: a relationship must come from a parsed `mod` declaration, a symbol's lexical containment, or a Cargo.toml dependency.
- Do NOT weaken an existing gate or criterion to land this.

## Measured today (why this spec exists)
- entities 1415 (23 crates) created by `scripts/pii-index-code.py`; relationships 0
- `pii memory link <id> <id> --rel contains` prints the ENTITY usage — the verb is grouped at cmd_ctl.rs:191 with no parser
- `pii memory search` on every query returned `"lanes":["bm25","vector"]`: the graph lane has never fired for code
