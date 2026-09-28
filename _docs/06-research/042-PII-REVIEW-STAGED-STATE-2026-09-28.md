# PII REVIEW — STAGED STATE FOR RESUME (2026-09-28)

<!-- This file is the ONE canonical copy of the pi-improved review's staged state.
     Everything else (the workspace resume register, pi-improved's HANDOFF, the project
     docs INDEX, the open-register row) carries a POINTER to this file, never a copy.
     Per the workspace SSOT rule: each fact in exactly one place; link, don't duplicate. -->

**One line:** the pi-improved claude+opus full review was **never dispatched**; everything is staged and the
exact command is below. Read this file before touching the pii review.

## What was requested (user, verbatim)

> *"i want one more. use our new skill 'opus-review/' and run it on the 'pi-improved' project, with the same
> commands you did with the amem project! i will check the report in the morning!"*
> … *"pii report ran TODAY!!!! not from fable not from day 18-09-2026! but from opus and TODAY! then it should
> derive from those fable reports! it should be ready to run when we finish the amem intervention!"*

So: an **Opus** review of **today's** tree, **derived from** the two 2026-09-18 fable reports (completing and
superseding them, not repeating them), producing a **detailed report** and **specs** — ready to drive the pii
intervention the moment the amem intervention finishes.

## The honest status: NOT RUN (twice)

Two dispatches were scheduled; **neither ran the review**.

| attempt | what happened |
|---|---|
| 07:40 job `c3da54bab186` | all three items died in ~20 s on the ROOT/IS_SANDBOX trap (`--dangerously-skip-permissions cannot be used with root/sudo privileges`). Its pii item produced only the 1-turn PONG probe: session `d255d913-9395-465c-92f8-a22497730c35`, **0 tool calls**. |
| 09:56 relaunch (manual) | item 1 (the amem re-review) ran; **item 2 (this) and item 3 logged `WINDOW SHUT — not starting`** — my own window guard refused with 4 minutes left before the 10:00 reset. |

**Proof that no pii report exists from 2026-09-28:** the only `-projects-pi-improved` claude session modified
today is `d255d913` (07:40, 123 KB, **0 tool calls**, text = `PONG55`/`PONG55`). Every file written today that
mentions pii traces back to *preparing* this review (the brief, the inventory, register rows) — none to a run.
The only pii review that exists on disk is **2026-09-18**.

## Everything staged and ready

| asset | path |
|---|---|
| **the brief** (derivation + gate commands + 8 decisions) | `/projects/pi-improved/_docs/06-research/041-BRIEF-full-review.md` |
| **the inventory** (18,929 lines; an appendix to look things UP, not to read) | `/projects/_tmp/pii-review/codebase-inventory.txt` (regenerate if `_tmp` is cleared) |
| the base to derive from | `/projects/pi-improved/_docs/06-research/031-fable-review-ground-truth-2026-09-18.md` and `032-fable-pii-review-2026-09-18.md` |
| the project's own reviewer entry point (8 decisions) | `/projects/pi-improved/_docs/06-research/030-OPUS-START-HERE.md` |
| the brief filed inside the project (survives `_tmp`) | `/projects/pi-improved/_docs/06-research/041-BRIEF-full-review.md` |
| the runner | `/root/.hermes/skills/research/opus-review/scripts/run-opus-review.sh` |

**Session to resume (REQUIRED — resume-first is the standing rule):**
`ef17e6d6-1741-42d6-87c6-16ffc775d231` — the session that produced the 2026-09-18 analysis, so the review
*derives from* its own prior work instead of re-deriving it (register: review→implement→re-review pipeline;
resume-first on all agents).

**Path convention — the two arguments differ on purpose:**
- **brief (INPUT)** → the project's `_docs/`, **durable**; a `_tmp` copy was reduced to a pointer so it cannot
  drift from this one.
- **workdir (OUTPUT)** → `/projects/_tmp/pii-review/`, scratch for results and logs. Repo rule: nothing needed
  to *start* lives there, so clearing `_tmp` loses no input. It will be recreated if absent.

## THE ONE COMMAND (nothing else to assemble)

```bash
bash /root/.hermes/scripts/pii-review-run.sh
```

`DRY_RUN=1` in front prints what it will do and spends nothing. It probes the window, dispatches in test
mode on the resumed session, proves the tree stayed clean, files the report into `_docs/` + amem, and
writes a commit-ready summary. **Exit 2 = window shut, nothing spent.**

## THE RESUME COMMAND (what the script runs, for reference)

```bash
cd /projects/pi-improved
REVIEW_MODE=test MAX_TURNS=200 MODEL=claude-opus-5-5 \
  bash /root/.hermes/skills/research/opus-review/scripts/run-opus-review.sh \
       /projects/pi-improved \
       /projects/pi-improved/_docs/06-research/041-BRIEF-full-review.md \
       /projects/_tmp/pii-review \
       --resume ef17e6d6-1741-42d6-87c6-16ffc775d231
```

- `REVIEW_MODE=test` is **required**: the whole point is that it *runs* the six gate commands
  (`fmt`, `clippy --all-targets -D warnings`, `build`, `test`, `doc`, `xtask lint-files`) — the gap the
  2026-09-18 run left open ("every cargo invocation was denied"). The runner now exports `IS_SANDBOX=1`
  itself, so this works under a scheduler too.
- The runner proves read-only afterwards (it prints whether the tree stayed clean).
- **Do not** use `claude -p` bare, and never `--resume` without a prompt (it is read as "continue a deferred
  session" and dies).

## After the run — the filing steps (in order)

1. **File the report** into the project's SSOT with a provenance header:
   `_docs/06-research/041-opus-full-review-<date>.md` via
   `bash <runner-dir>/file-opus-review.sh <repo> <result.md> <dest_rel>`.
2. **Index it** in `/projects/pi-improved/_docs/INDEX.md`.
3. **Point `030-OPUS-START-HERE.md` at it** (a dated banner at the top) so the intervention finds it first.
4. **Store the FULL text in amem** under `pii/review/full-codebase-<date>/` (manifest + parts), not a summary —
   the standing rule is that every implementation aspect lives in amem.
5. **Commit** — the run leaves the tree dirty by design; pii's rule requires a HANDOFF update with every commit.
   The run leaves the tree dirty; the commit is yours to make (this file names the exact add/commit set).
6. **Turn its ranked candidates into specs** (they are written implementation-ready, each with a checkable exit
   criterion, in the shape of 032's list) — that is what makes it "ready to run when we finish the amem
   intervention".

## Constraints that apply (from the project and the workspace)

- `_repos/` is **read-only**; `specs/` is planner-only; tests belong under `_tmp/`, never `/tmp`.
- Workspace rules: **never `rm -rf`** (a guard must fail SAFE on an empty value); workers never commit.
- The review must **not** edit tracked files, commit, deploy or restart anything.
- Provider discipline: Claude Code + Opus only for this lane — no ollama-cloud, no nvidia, no copilot.
- If the window closes mid-run: stop and record the exact reset time; **never blind-retry**.

## What "done" looks like

- A filed report in `_docs/06-research/041-…md` with a provenance header naming model/mode/brief/HEAD.
- Gate results for all six commands, with raw output and exit codes (not summaries).
- An explicit statement of which of 031/032's findings are now **fixed / still standing / wrong**.
- A ruling on the **eight decisions** in `030-OPUS-START-HERE.md`, each with evidence and what would change its
  mind — including the project's veto condition ("if the code graph cannot make a gate measurably stronger, it
  does not belong in pii").
- Ranked, implementation-ready candidates → the next spec batch.
- The full text in amem; the INDEX and the entry point pointing at it.
