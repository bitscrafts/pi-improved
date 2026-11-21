# Disk ceilings, and every path outside git

**Filed 2026-11-20. Owner's rule, verbatim: *"precisamos dos ficheiros todos rastreados! precisamos de analise,
verificar a importancia dos arquivos, reusabilidade, e rastreio!"***

`git status` says the tree is clean — 0 untracked, non-ignored files — and that is exactly the problem this document
answers: **clean is not the same as accounted for.** 193 GB sat behind 20 `.gitignore` rules with nothing saying what
they were, whether they could be regenerated, or what stopped them growing.

## The three states a path can be in

```
TRACKED      in git: source, specs, docs, tests, fixtures, configuration that ships. Has a commit and a reviewer.
CATALOGUED   deliberately not in git, and named in the table below with: what it is, why it cannot be tracked, how it
             is recreated, and its ceiling. Anything not in the table is a DEFECT, not a judgement call.
DEFECT       in the tree, neither tracked nor catalogued. Includes artefacts a run leaves behind.
```

The rule that follows: **a path that cannot be tracked must be catalogued with a ceiling, and a ceiling without
enforcement is decoration.** The enforcement is `scripts/checks/tree-is-catalogued.sh` (spec 436): it lists every
ignored path, fails on one that is not in this table, and fails when a catalogued path is over its ceiling.

## The catalogue (measured 2026-11-20)

| path | size | what it is | recreated by | ceiling | importance |
| --- | --- | --- | --- | --- | --- |
| `_tmp/gate-target/` | 81 G | the gate's warm build cache, one sub-dir per step (`test-gate`, `clippy-gate`, `build-gate`…) | the next gate run (cold: ~40 min) | **30 G**, pruned to the steps in use | high — the difference between a 20-minute and a one-hour gate |
| `_tmp/_trash/` | 73 G | everything moved aside by "never delete, move" | nothing — it is only ever written | **5 G**, rolled at 7 days | none — waste with a grace period |
| `_tmp/claude-batchC/` | 25 G | 221 484 files: per-run briefs, logs and captured output of the worker lane | re-running the lane | **3 G**, rolled at 14 days / 20 runs | medium for a fortnight (the briefs are in `specs/`), none after |
| `_tmp/` side caches | ~8 G | `tools-gate`, `phase4.7`, `devbox-test`, `xtask-target`, `head-builds-target`, `target-280`, `verify-*`: caches of runs that already ended | their own command | **0** — one warm cache is enough | none |
| `target/` | 37 G | build output of the workspace | `cargo build` | **0** in a clean tree (a developer's own is their choice) | none — the gate builds in `_tmp/gate-target` |
| `.pidag/` | 1.3 G | pidag's run-history database and export | re-running pidag (history is lost) | **200 M**, pruned | low — the runs that mattered are in `specs/` and the commits |
| `_repos/` | 940 M | clones of upstream repositories used as reference | re-clone | 1 G | high while reading them: they are the provenance for comparisons |
| `.pii-sessions/` | 5.0 M | the daemon's own session store | the daemon | 50 M | high — product state, not scratch |
| `.pii/` | 260 K | project-level pii configuration and state | hand-written | 1 M | high — **should be TRACKED** once checked for credentials (spec 437) |
| `.claude/ .hermes/ .minilam/ .pytest_cache/` | <200 K each | tool state and caches, none of it ours | the tools | 1 M each | none |
| `crates/*/_tmp/` (17 dirs) | small | per-crate test scratch, created by the tests themselves (the `lint-scratch` rule requires it) | the tests | 50 M per crate | none, but the test that makes it must clean it |
| `crates/**/default_*.pro` | transient, hundreds at 664 K | **profiler output written into the source directories** | a profiling run during tests | **0 — defect** | none; it must never exist here (spec 435) |

The last row matters beyond disk: those files appear and disappear while tests run, they are ignored (so `git status`
stays clean), and a run that leaves one behind surfaces as an untracked file in `leftover.sh owner` and **stops the
queue**. `lint-scratch` covers temporary *directories*, not stray output *files*.

## Importance, reusability, traceability — what the table is for

The point is not the ceiling; it is the middle columns.

- **Importance.** `_repos/` and `_tmp/gate-target/` are high because they change what a person can do in the next hour
  (compare against upstream; gate in 20 minutes instead of 60). `.pii-sessions/` and `.pii/` are high because they are
  *product* state: losing them loses behaviour, not time.
- **Reusability.** Everything under `_tmp/` is single-use by definition except the one warm cache — that is what `_tmp`
  means. `_repos/` is reusable indefinitely. The distinction decides the ceiling, not sentiment.
- **Traceability.** A tracked file has a commit and a reviewer. A catalogued file has this table and a ceiling. A defect
  has neither, and is the only state that can hide. The profiler output is the proof: 8 GB of it existed, invisible to
  `git status`, and nothing in the repository would have noticed it grow.

## Keeping it small

The windows are enforced by `pii disk clean` (spec 438): `target/` cleaned, side caches of finished runs removed,
`_tmp/_trash` rolled at 7 days, `_tmp/claude-batchC` at 14. It refuses any path that is not inside `_tmp/`, refuses the
warm gate cache and the queue's own state, and prints sizes before it acts. Until that subcommand exists, the same
procedure is run by hand from this document — and it is deliberately **not** kept as a script, because a rule that lives
in a script someone must remember to run is the "runtime belongs in the binary" drift this repository names in
`_docs/04-development/runtime-belongs-in-the-binary.md`.

Measured once, 2026-11-20, running that procedure by hand: `_tmp/claude-batchC` 25 G → 1 G (214 079 files older than 14
days), `_tmp/_trash` 5 448 entries removed, `cargo clean` 37.6 GiB — **20 GB free became 81 GB** in one pass. What did
*not* free was `_tmp/_trash`: its 73 GB was all younger than 7 days, and the two biggest entries were a stale
`test-gate` (16 G) and dozens of `lane-pin-head` copies (200 M each) — build artefacts wearing a work-shaped name. A
retention window cannot help against a trash that is refilled faster than it expires; that is a writer to fix (spec 435
is the same class), not a window to shorten.

The gate's own floor is the other half: the cycle refuses to gate below **20 GB** free, and the coverage step alone
writes **37 GB**. A gate that starts at 20 GB fails six steps that look like code defects (`No space left on device`
inside two tests, two discipline lints and coverage). Spec 438 raises the floor to 60 GB and names the reason.

## The target

```
source + docs + specs      ~1.5 G   tracked
one warm gate cache        30 G     catalogued, pruned to the steps in use
trash                      5 G      catalogued, 7 days
worker logs                3 G      catalogued, 14 days
session + config           55 M     tracked or catalogued, product state
_repos                     1 G      catalogued, reference
                          ─────
                           ~40 G    instead of 238 G
```

The difference is not deleting more. It is putting a ceiling on three things that had none, and making every path
traceable to one row of one table.