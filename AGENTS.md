# Pii — guide for agents and contributors

Pii ("pi-improved") is a Rust workspace: a terminal coding agent, an assistant daemon, an automation engine and a learning layer built
around one event-sourced daemon. Four binaries ship: `pii` (from `crates/pii-cli`, the main entry point, which embeds the daemon),
`pii-mcp`, `pii-web` and `pii-evolve`. The workspace is 22 crates under `crates/` plus `xtask`.

Start with [`README.md`](README.md) for the shape of the system and [`HANDOFF.md`](HANDOFF.md) for the current state. Documentation is
indexed in [`_docs/INDEX.md`](_docs/INDEX.md).

## How a change is made

Every change starts as a numbered spec in [`specs/`](specs/). A spec has a card line under its title (`#: depends= status= block=`), a
TDD contract, and exit criteria that are runnable commands.

- A criterion must exit 0 when the work is done and non-zero before it. One that already passes is not a criterion: put it under
  Guardrails. `bash scripts/red-audit.sh <row>` reports how many already pass.
- A spec becomes a row in `scripts/autonomy/plan.tsv` (id, spec file, brief, title, criteria, paths, lane).
  `scripts/autonomy/cycle.sh` dispatches one row, gates the result and commits it; `scripts/autonomy/state.py` holds each row's status.
- The one who writes a spec does not implement it, and an implementer never edits `specs/` and never commits. The work is left in
  the tree for the gate and the reviewer.
- A cycle takes a lock about every five minutes. Do not edit the tree while one runs: take the hold (`scripts/autonomy/hold.sh`) or wait.

Outside contributions go through issues and pull requests: see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## The gate

- `bash quality-gate.sh --routine` runs the steps of [`gate-steps.txt`](gate-steps.txt) (22 steps: formatting, clippy with warnings denied,
  a workspace build, tests, doc tests, the coverage floor and the repository's own lints) in this checkout. `gate-steps-quick.txt` is the
  5-step list used per item.
- `cargo run -q -p xtask -- fresh-gate --worktree` runs the same list in a clean export. When the two disagree, believe the fresh
  one: a gate can be green in a tree and red in a clone. Manual: [`_docs/02-guides/quality-gate-and-fresh-tree.md`](_docs/02-guides/quality-gate-and-fresh-tree.md).
- A cold gate needs about 40 GB of build output; the cycle refuses to gate below 20 GB free on the build volume.
- `.github/workflows/checks.yml` runs the cheap half of the gate on every push and pull request. `gate.yml` is the full gate, run on demand.

## Rules

- A source file stays under 450 lines. Split into a folder module; do not raise the limit or baseline a new file.
- `unsafe_code = "forbid"` for the whole workspace (`Cargo.toml:60`), with no per-crate override.
- Tests keep their scratch under `_tmp/` and create it themselves: `pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "prefix-")`,
  or an explicit `create_dir_all` in a crate that does not depend on `pii-store`. A bare `tempdir_in(.../_tmp...)` fails the
  `lint-scratch` step.
- A test is written first and must fail for the right reason. A rule that skips or suppresses something needs a near-miss case that must
  still take the normal path.
- Never weaken a check, lower a floor or skip a step to make a change pass.
- Never `rm -rf` existing work and never discard changes with `git checkout --`: move things aside. Credentials come from environment
  variables and are never written to a file, a log or a commit.
- Commit messages describe the change and are written under your own identity.
- Update `HANDOFF.md` when a change alters what is open or how the queue works.
