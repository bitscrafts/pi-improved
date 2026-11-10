# Validate worktree `signature` and Scope-Check the Joined Path

- **Project**: `<repo>`
- **Crate**: `pii-tools`
- **Priority**: HIGH — security-relevant. Path-traversal-adjacent issue
  in the one tool that creates directories and shells out, bypassing
  the crate's own capability-gate layer. Sourced from fable review
  finding 6, `_docs/06-research/032-fable-pii-review-2026-09-18.md`
  ("Follow-up" section) — independently re-verified by reading the
  code directly before writing this spec.

## Overview

`crates/pii-tools/src/worktree.rs`'s `WorktreeTool::call` (line ~54)
takes `signature` straight from the tool-call JSON with only a
"must be a string" schema constraint (`schema()` at line ~39 says
`"description": "heal signature (blake3 hex)"` — description, not
enforcement). It is then used unvalidated:

```rust
let sig = args.get("signature").and_then(Value::as_str)
    .ok_or_else(|| PiiError::Invalid("worktree requires 'signature' string".into()))?;
let dir = self.workspace.join("_tmp").join("heal-worktrees");
...
let wt = dir.join(sig);
```

`PathBuf::join` on an ABSOLUTE `sig` replaces the entire base path
(Rust's own documented `Path::join` behavior); a relative `sig`
containing `..` components is appended literally, walking back up the
tree. The resulting `wt` is then passed to `git worktree add -b
heal/<sig> <path> HEAD` via `/bin/sh -c` (verify current line numbers
— this logic is further down the same function). `shell_quote`
prevents shell-metacharacter injection but does nothing about the
path itself already pointing somewhere unintended.

Every other tool in this crate (`fs.rs`, `search.rs`, `bash.rs`) routes
path access through `Ctx::fs()` / `FsCap` (`pii-core/src/fscap.rs`),
which canonicalizes and scope-checks against the workspace boundary.
`worktree.rs` never calls `Ctx::fs()` at all — it uses `std::fs`/`std
::path` directly, so the capability-layer guarantee the rest of the
codebase relies on does not apply here.

This is reachable from any agent node that lists `worktree` as an
available tool, and from every MCP client, since `pii-mcp/src/main.rs`
registers `ToolSet::builtin` wholesale (verify this claim against the
current file before assuming it, since it's about a different crate's
registration behavior).

Ground truth (verify yourself, re-read the file — it has grown since
this spec was drafted):
- `crates/pii-tools/src/worktree.rs` — `WorktreeTool::call`, the
  schema definition, and the `git worktree add` shell invocation
  further down the same function.
- `crates/pii-core/src/fscap.rs` — the canonicalize-then-scope-check
  pattern (`FsCap`) that every OTHER tool in this crate uses; follow
  the same shape of check here even though this isn't a full `FsCap`
  integration (this spec is scoped narrower — see Guardrails).

## Requirements

### Functional

- R1. Reject `signature` values that are not exactly 8-64 lowercase
  hexadecimal characters (`[0-9a-f]{8,64}`) with
  `PiiError::Invalid("worktree: signature must be 8-64 lowercase hex characters")`
  — this matches the doc comment's stated intent ("blake3 hex") without
  requiring a specific blake3-output length, since callers may pass a
  truncated/prefix signature.
- R2. AFTER constructing `wt = dir.join(sig)` (post-R1 validation),
  assert the result is still contained within `dir`: canonicalize is
  not strictly needed here since R1 already rules out `..` and `/`
  characters (hex-only regex makes path escape structurally
  impossible), but add the containment assertion anyway as
  defense-in-depth — if `wt.strip_prefix(&dir).is_err()`, return
  `PiiError::Invalid("worktree: signature resolves outside worktree directory")`.
  This makes the invariant explicit and testable rather than relying
  solely on the regex being correct forever.
- R3. Apply the identical validation to every other site in this file
  that reads `signature` from args (there may be more than the one
  `call` site — e.g. a `heal-prune`-adjacent tool if one exists in the
  same file; search for other `.get("signature")` occurrences before
  assuming there's only one).

### Non-Functional

- N1. A signature that already matches the regex (the common,
  legitimate case — real blake3 hex output) must behave byte-identically
  to today: same directory, same branch name, same git invocation.
- N2. Do NOT touch `fs.rs`/`search.rs`/`bash.rs` or `Ctx::fs()`/`FsCap`
  — this spec fixes `worktree.rs` locally with input validation, it
  does NOT migrate the tool to route through the capability layer
  (that would be a larger, separate refactor with its own tradeoffs
  around whether worktree provisioning should be capability-scoped the
  same way file I/O is — flag this as a follow-up idea in your report,
  do not attempt it here).

## Architecture

Described inline in Overview above (file paths and function names given there constitute the architecture context for this change).

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| `worktree_rejects_path_traversal_signature` | `signature: "../../../etc"` | `Err(PiiError::Invalid(_))`, no directory created outside `heal-worktrees/` |
| `worktree_rejects_absolute_path_signature` | `signature: "/tmp/evil"` | `Err(PiiError::Invalid(_))`, no directory created at `/tmp/evil` |
| `worktree_rejects_non_hex_signature` | `signature: "not-hex-chars!"` | `Err(PiiError::Invalid(_))` |
| `worktree_rejects_too_short_signature` | `signature: "abc"` (< 8 chars) | `Err(PiiError::Invalid(_))` |
| `worktree_accepts_valid_hex_signature` | `signature: "a1b2c3d4e5f6"` (valid length/charset) | Succeeds exactly as before this change; worktree provisioned at `<workspace>/_tmp/heal-worktrees/a1b2c3d4e5f6` |
| `worktree_containment_check_is_defensive` | (unit test on the containment logic directly, not through the full tool call, if the check is factored into a helper function) | confirms `strip_prefix` catches any path escaping `dir`, independent of the regex |

## Exit Criteria

- [ ] `cargo test -p pii-tools -- worktree`
- [ ] `cargo test -p pii-tools -p pii-mcp`
- [ ] `cargo build -p pii-tools`
- [ ] `cargo clippy -p pii-tools -- -D warnings`
- [ ] `cargo fmt --check`
- [ ] `cargo run -q -p xtask -- lint-files`

## Guardrails

- Do NOT modify `fs.rs`, `search.rs`, `bash.rs`, or `pii-core/src/fscap.rs`
  — this is a narrowly-scoped input-validation fix to `worktree.rs`
  only.
- Do NOT migrate `WorktreeTool` to route through `Ctx::fs()`/`FsCap` —
  that is explicitly out of scope (see N2). If you believe this
  migration is necessary to actually close the issue, STOP and report
  that recommendation rather than implementing it unasked.
- Do NOT modify any file under `specs/`.
- Do NOT touch the `git worktree add` shell-invocation logic itself
  (the `shell_quote` usage) — that part is already correct for its
  concern (shell-metacharacter safety); this spec addresses the
  path-construction concern only.
- No emojis, no AI-tool mentions in any comment or commit message.
- If `signature` is read from more than one location in this file (per
  R3) and any of those call sites has a genuinely different validity
  constraint (e.g. a different expected length), STOP and report
  rather than forcing one regex to fit all cases.
