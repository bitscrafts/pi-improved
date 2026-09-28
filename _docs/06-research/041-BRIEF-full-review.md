# CONTINUATION — pi-improved: the full analysis you started, finished for real (resume of this session)

You analysed this repository in **this same session** (`ef17e6d6-…`, 2026-09-18). Your brief then was explicit
that the run it followed had a hole, and that yours would close it:

> *"That run had a real gap: every cargo invocation was denied by tool permissions, so items 3 and 5 of the original
> brief (actually running clippy/tests/fmt) went unfulfilled, and analysis concentrated on only 4 of 20 crates …
> This run fixes both gaps."*

This brief asks you to **verify whether those two gaps are actually closed** — by running everything again, now,
against the current tree — and then to answer the **eight decisions the project has been holding for you**.

## THE BASE YOU MUST DERIVE FROM (do not redo; complete and supersede)

Two reports already exist in this repo and are the **starting point** for your analysis — read both before
anything else, and treat your job as *continuing and superseding* them, not repeating them:

- `_docs/06-research/031-fable-review-ground-truth-2026-09-18.md` — the ground truth: crate map (name → LOC →
  deps), the file-size policy measured with the gate's own ruler, test-coverage snapshot, and the list of
  **deliberately-deferred items you must NOT re-propose as new findings**.
- `_docs/06-research/032-fable-pii-review-2026-09-18.md` (**33 KB, 571 lines**) — the review itself: 4 findings in
  pass 1, then a pass-2 live-execution + full-crate follow-up (findings 5–10), 12 **ranked candidate
  improvements**, and the honest negatives it recorded.

**What was done with them:** candidates 1–4 and 6–11 became **specs 62–69**, all committed with the gate green
(`883d768`, `890c285`, `fe35c37`, `2e47219`, `6871616`, `c47de9d`, `75606c4`, `55a8328`; summary in `ef4d782`).
Candidates 5 and 12 were deliberately not specced.

**Your job on top of that base:**

1. **Verify the implementations, not the claims.** Specs 62–69 are committed but their individual verification
   status today is unproven. For each, read what it changed and confirm the behaviour on the CURRENT tree.
2. **Supersede the findings where the tree has moved** — say which of 031/032's findings are now fixed, which
   still stand, and which were wrong.
3. **Carry the unfinished work forward.** 032's own "not verified this run" list, and anything in 031's deferred
   list that the intervening work did not touch.
4. **Produce the same two deliverables this project expects** (see "The report" below): a detailed filed report,
   and the **ranked candidate improvements** that would become the next specs — implementation-ready, each with
   an exit criterion a validator can check, in the shape 032 used.

## The state of the tree today

- Repo `/projects/pi-improved`, branch `master`, **HEAD `4a0a2db`**, tree clean.
- `cargo` works in this session: `PATH` includes `/usr/local/cargo/bin`, and the Rust toolchain is installed.
- **18,929-line inventory** of every crate, file, surface and doc: `/projects/_tmp/pii-review/codebase-inventory.txt`
  — an appendix, for LOOKING UP a file you are already interested in. **Do not read it end to end.**
- 20 crates, ~35,100 lines of `src/`. Largest: `pii-server` 7,821 · `pii-graph` 4,968 · `pii-store` 3,114 ·
  `pii-cli` 2,880 · `pii-net` 2,359 · `pii-core` 2,214 · `pii-model` 1,960 · `pii-tools` 1,954 · `pii-proto` 1,456.
- 85 spec files, 40 research/design docs, and a 33 KB `HANDOFF.md`.

## STEP 1 — run all six commands, verbatim, and paste raw output

This is the same list your 2026-09-18 brief carried. Run each one and report the **exit code and the raw output**
(exact `test result:` lines per binary; full text of any failure, not just its name):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace --lib --bins --tests
cargo test --workspace --doc
cargo run -q -p xtask -- lint-files
```

If any invocation is denied or fails for an environment reason, **say so explicitly and try an alternative
before giving up** (write output to a file, then read it) — do not silently skip. A gate that silently did not run
is the failure mode this whole exercise exists to catch: this project's own `HANDOFF.md` records a case where a
stale binary made the gate report false errors, and its earlier review had to be repeated precisely because the
commands were never run.

## STEP 2 — the crates the earlier passes skipped

The 2026-09-18 run concentrated on `pii-store`, `pii-model`, `pii-graph`, `pii-server`. Whether its follow-up
closed the gap for the other crates is part of what you are verifying. Check specifically:

- **`pii-cli`'s command files** — `cmd_ctl.rs`, `cmd_daemon.rs`, `cmd_run.rs`, `cmd_sandbox_exec.rs`, `cmd_soak.rs`,
  `cmd_sys.rs`, `cmd_tui.rs`, `cmd_worker_exec.rs`: do they carry inline `#[cfg(test)]` coverage or are they
  genuinely untested? Answer with grep/test output, not impression.
- **`pii-web`** — two tests for what it does: adequate, or a gap?
- **Any crate where Step 1 surfaced something concrete** — follow it to the file and say what it means.

## STEP 3 — the eight decisions the project is holding

The project maintains its own reviewer entry point,
`_docs/06-research/030-OPUS-START-HERE.md` (updated 2026-09-19). **Read it — it is written for exactly this
review.** It asks for a decision on each of:

1. **Coding-harness graph shape** — fork `graphs/starter/verify-fix.toml`, add multi-criteria validation and
   model-escalation-on-retry. Approve, or a different shape?
2. **F-13 scope** — implemented as a module reorg rather than a full 3-crate split. Right call, retroactively?
3. **Modularity ("microservices in a single binary")** — worth a design doc + prototype, or too early?
4. **Chromecast module** — scope as a project or defer? Sub-question: control is nearly free, but discovery needs
   UDP multicast which `pii-net` blocks unconditionally — is auto-discovery worth widening the SSRF boundary, or is
   a named-device registry the whole feature?
5. **`xtask reachability-check`** — upgrade from string search to a real call graph?
6. **Should pii host a code-knowledge graph as a native service, and where?**
7. **Should the coding harness consume that graph?**
8. **`_docs/06-research/` reorganisation** (28 files, flat-by-number) — low stakes, your call.

For each: a **decision**, the evidence you relied on, and what would change your mind. The project states a
**veto condition** you should hold it to: *"if the code graph cannot make a gate measurably stronger, it does not
belong in pii."* It also records an assistant recommendation that **pidag should become a constrained subset of
pii's schema, not the reverse** — confirm or override it, with reasons.

## Constraints (the project's own hard rules — respect them)

- **`_repos/` is read-only** (five reference clones). Never modify it.
- **`specs/` is read-only for workers**; spec authoring is planner work. You are analysing, not implementing.
- **Tests belong under `_tmp/`, never `/tmp`.** The 450-line file gate, traits-first, and
  `#![forbid(unsafe_code)]` are workspace-wide.
- **No `rm -rf`. No AI-assistant mentions in commit messages.**
- Do **not** edit tracked files, commit, push, or change dependencies. A previous session of this project had a
  run whose `cargo` calls were all denied — if that happens, report it instead of working around it silently.

## The report

1. **Gate results** — a table: command → exit code → one line of interpretation, with any failure's full output.
2. **Whether the two gaps from 2026-09-18 are closed**, and if not, exactly which part is still open.
3. **The eight decisions**, each with its evidence table (`file:line` or command), marked **CONFIRMED** (you ran or
   read it) or **INFERRED**.
4. **New defects found** — severity, `file:line`, and the command that demonstrates each.
5. **A NOT VERIFIED list** — what you could not check and why.
6. The **single next most valuable change** for this project right now.

Mark every claim. Prefer a command's output to a reading of the source, and prefer either to an impression.
