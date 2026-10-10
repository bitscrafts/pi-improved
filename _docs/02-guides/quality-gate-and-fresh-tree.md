# The quality gate: eight steps, two trees, and one rule about scratch

**Audience**: anyone (human or agent) changing this repo, and especially anyone who will
claim "the gate is green". This is the operating manual for `quality-gate.sh`, for the
`fresh-gate` variant, and for the two project-specific lints that enforce conventions the
compiler cannot.

Source of truth for the step list is the script itself (`quality-gate.sh`) — this document
explains *why* each step exists and what it cannot see. When they disagree, the script wins.

---

## 1. The eight steps (and one prerequisite)

| step | what it runs | why it exists |
|---|---|---|
| `fmt` | `cargo fmt --all --check` | zero-tolerance formatting |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` in its **own** `CARGO_TARGET_DIR` | links every target; isolated so its `--all-targets` rlibs cannot poison the doctest step (F-01/E0463) |
| `clippy (llamacpp)` | `cargo clippy -p pii-model --features llamacpp --all-targets -- -D warnings`, own target dir | spec 89/N4: `llamacpp` is off in every other step, so without this the in-process llama provider and the tests carrying `required-features = ["llamacpp"]` would be compiled by **no** gate step |
| `build` | `cargo build --workspace` | `pii-worker`'s supervisor tests spawn the real `pii` binary at a fixed `target/debug/pii` path |
| `test (lib/bins/tests)` | `cargo test --workspace --lib --bins --tests` | the suite proper |
| `test (doc)` | `cargo test --workspace --doc` | doc examples; run separately because doctests build libs *without* `cfg(test)` |
| `lint-files` | `cargo run -q -p xtask -- lint-files` | file-size ratchet + spec/doc hygiene checks the compiler cannot express |
| `lint-scratch` | `cargo run -q -p xtask -- lint-scratch` | spec 96: fails on a test that uses a crate-local `_tmp` directory it never creates |
| `reachability-check` | `cargo run -q -p xtask -- reachability-check` | spec 72/87: proves the named seed symbols still have a real, non-test, non-comment caller |

A step is a step only if it can **fail the gate**. `lint-scratch` and `reachability-check`
are enforcement, not advice: they exit non-zero and set the gate's status.

## 2. Two trees: the developer tree and a fresh export

```bash
bash quality-gate.sh                 # in the repo, uses the working tree
cargo run -q -p xtask -- fresh-gate --worktree
cargo run -q -p xtask -- fresh-gate --overlay crates/x/src/y.rs=/path/to/fixed.rs
```

`quality-gate.sh` runs in **your** tree, next to whatever state your last run left behind.
`fresh-gate` exports `HEAD` with `git archive` into a throwaway directory and runs the gate
**there**, which is the CI model: a clean clone with no leftovers.

They disagree, and when they do **`fresh-gate` is the one to believe**. Measured three times
in one day (specs 83, 85, 86, 96): tests that use a crate-local `_tmp` directory they never
create pass in a developer tree where an earlier run left the directory, and fail in an
export — a gate that is green on exactly one machine. The repo gate cannot see this by
construction; that is why the second tree exists.

Practical use:

- **`--worktree`** overlays your uncommitted work, so you can check a change before
  committing it.
- **`--overlay src=dest`** substitutes one file — the way to test a deliberately mutated
  file without touching the tree.
- **Run it for any change that touches tests.** It costs a full cold workspace compile
  (minutes) and ~8 GB of disk.

## 3. Tests own their scratch: `tmp_scratch`

The project convention (AGENTS.md) is that test scratch lives under `_tmp/`, never `/tmp`.
The enforceable form of that rule:

```rust
// the only sanctioned way to get a test scratch dir
let dir = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "my-test-")?;
```

It creates the crate-local `_tmp` **and** the temp dir inside it, so a test can never depend
on a directory that a previous run happened to leave behind. `lint-scratch` fails the gate on
a bare `tempdir_in(.../_tmp...)` with no `create_dir_all`/`tmp_scratch` before it in the same
function. Crates that do not depend on `pii-store` use an explicit `create_dir_all` instead.

If a test needs a scratch dir and you are reaching for `tempfile::Builder::new()...`, use the
helper. The lint will tell you anyway.

## 4. Acknowledgements and limits

- **`fresh-gate` keeps at most ONE failed export** and **refuses to start below a free-space
  floor** (`FRESH_GATE_MIN_FREE_MB`, default 2048). Both guards exist because five 8 GB
  exports once took the shared volume to 0 bytes and silently killed a dispatched worker
  mid-edit. If a run reports a failure, the failing test NAMES are the evidence — record them
  and delete the export.
- **`fresh-gate` is not part of `quality-gate.sh`.** It recompiles the workspace and cannot
  run on every commit. Run it deliberately, for test-touching changes.
- **A green gate is not a correct change.** It says the checks passed, not that the change
  did what the spec asked. Verify the diff, and for a landed fix, mutate it back and watch
  the test go red (see `_docs/06-research/046-`, `047-`, `049-`).
- **Do not "fix" a red fresh-gate by creating the directory in the gate script.** That hides
  the class again; the point is that tests create what they need.

## Provenance

Written 2026-10-09 after the 045 re-review arc (specs 83-96). The step table matches
`quality-gate.sh` at `d4084ff`. Evidence for the two-tree model, the class it catches and the
enforcement lints:
`_docs/06-research/046-mutation-verification-batch-cd-2026-10-09.md`,
`051-scratch-helper-and-sweep-2026-10-09.md`, `050-inline-methods-measured-2026-10-09.md`.

## The two modes, measured (2026-10-10)

`cargo xtask fresh-gate` builds a temp export and runs the whole gate inside it. `xtask/src/fresh_gate.rs`
makes the export in two steps, and knowing which one you asked for decides what the result proves:

```rust
export_head(&root, &dir)?;            // git archive HEAD | tar -x   -> the COMMITTED tree
if opts.worktree { overlay_worktree(&root, &dir)?; }   // --worktree: copy the working tree's edits on top
```

* **plain `fresh-gate`** — the export is **exactly HEAD**. This is the form that verifies a COMMIT, and the form
  to use while another session is working in the repo: their edits cannot reach it.
* **`fresh-gate --worktree`** — HEAD **plus** the current uncommitted state of every git-visible path (a snapshot
  taken at run start). This is the form that checks "my work as it stands", and it is the form that a worker can
  legitimately use to verify its own uncommitted changes.
* **The trap** — a `--worktree` run while someone else is mid-edit silently includes THEIR half-finished file.
  Measured: spec 115's worker was editing `crates/pii-cli/src/cmd_soak.rs`, and a `--worktree` run of spec 114's
  commit failed `build`/`clippy`/`test`/`lint-observability` with `error[E0277]: RpcError doesn't implement
  std::fmt::Display` — an error in nobody's committed code. A one-line check before trusting a `--worktree`
  failure: `git status --porcelain` — if it is non-empty and the files are not yours, the run measured someone
  else's work in progress.

Retention: a PASS removes the export; a FAIL keeps one and removes older ones. Disk: `cargo clean` inside a kept
kept export reclaims its target (measured 13.0 GiB) without deleting the directory.
