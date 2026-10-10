# The learning loop, stages 1-2: extraction (spec 123) and verification (spec 124)

> **What this is, stated plainly.** Extraction reads the trajectory files the daemon
> already writes and **proposes** *candidate lessons*, each citing the runs and files that
> justify it. It is deterministic and offline: **no model call, no network**. It does
> **not act** — no config change, no threshold move, no code mutation comes out of a
> candidate. Stage 124 (verifier) and 125 (promotion) do not exist yet; until they do,
> a candidate is a note for a human.

The crate is `pii-learn`. It is its own crate because it needs only `pii-observe` (the
trajectory format and the event names), which keeps it off the engine's dependency path;
the daemon links it solely for the read-only `learn` health seam.

## Running it

```
pii learn scan [--traj <root>] [--dir <store dir>] [--min-runs N] [--outlier-factor N]
               [--max-files N] [--max-file-bytes N]
pii learn list [--dir <store dir>]
pii learn show <id> [--dir <store dir>]
```

`--traj` is the trajectory root (the directory that holds `trajectories/`; default
`$PII_TRAJECTORY_DIR`). `--dir` is where candidates are stored, under `learn/` (default:
the daemon's runtime dir). `scan` is a **separate, short-lived process**: it never runs
inside the daemon, so it cannot block it. Run it from cron or by hand, under `nice` if the
directory is large. The daemon only *reads* the small `learn/status.json` a scan leaves
behind, for `pii health` (the `learn` subsystem) and `pii metrics`.

Candidates are files `learn/candidates/<id>.json`, read-only to the graph engine; nothing
in the daemon consumes them.

## The extractors and their thresholds

| kind | fires when | default | why |
|---|---|---|---|
| `repeated_failure` | the same `(graph, node, error class)` failed in at least `min_runs` distinct runs | `min_runs = 3` | two runs can be coincidence; three is the smallest "repeated" |
| `stuck_loop` | a graph has at least `min_runs` runs and **every** one failed at the same `(node, class)` | `min_runs = 3` | one success proves the graph can pass, so it is not stuck |
| `limit_hit` | a registered limit/refusal event appears in at least `min_runs` distinct runs | `min_runs = 3` | same reasoning as above |
| `cost_outlier` | a run's summed node duration (or file bytes) is at least `outlier_factor` × the graph's lower median, in a graph with at least `outlier_min_runs` runs, **and** above an absolute floor | `outlier_factor = 5`, `outlier_min_runs = 5`, floor `1000 ms` / `4096 bytes` | a median needs a few runs; the floor stops a ~0 median from flagging everything |

The limit events are the registered names, not a second taxonomy: `trajectory.truncated`
(115), `wasm.memory_limit`, `wasm.timeout`, `wasm.fuel_exhausted`, `wasm.refusal` (122),
`worker.request.refused` (120), `run.start.refused`. Only events that carry a run id reach a
trajectory, so a limit event emitted without a run id is invisible to extraction.

A failure with no failing node (a scheduler-level failure, e.g. an unsatisfiable join) is
recorded at node `-`, shown in claims as "the run terminal".

## Candidates, and when they are refused

A candidate has a content-derived `id` (`c-` + 16 hex of blake3 over kind, dedupe key, run
ids and counts — the same evidence gives the same id; paths are left out so moving a
trajectory root does not rename a lesson), a `kind`, a one-line `claim`, `evidence` (run
ids, artefact paths **relative to the scan root**, and the counts that triggered it) and a
`dedupe_key`. `pii learn show` prints the scan root so a path can be opened.

A candidate with no run id, no artefact path, no counts, or a blank claim is **refused,
not emitted**; the refusal is counted (`learn.candidate_refused`, `pii_learn_refusals_total`).
Every extractor builds candidates through one constructor, so there is no path around it.
A claim is composed only from numbers and closed-vocabulary tokens that are also in its
`counts`, so a reader holding only the evidence can check it. It cites paths and counts and
never contains a trajectory dump. Evidence lists at most 50 runs; the true number is in
`counts.runs`.

## The substrate is data, never instructions

A trajectory can contain anything a tool, a model or a user produced, including text like
"ignore previous instructions". Extraction never lets that reach a decision or an output:

* free text (error messages, run detail, node output) is **never copied**; an error is
  reduced to one of a closed set of classes (`timeout`, `rate_limited`, `memory`, `fuel`,
  `denied`, `not_found`, `invalid`, `io`, `other`) by keyword, and the text is dropped;
* node and graph names are reduced to `[A-Za-z0-9._:/-]`, at most 64 characters, so they
  cannot carry a sentence;
* `wasm.refusal` reasons are matched against the documented list, anything else is `other`;
* there is no model in the loop to be persuaded.

The control is a test: a fixture whose error, detail and node output carry a hostile
string must produce byte-identical candidates to the same fixture with neutral text
(`injection_control_hostile_text_changes_nothing`), and a mutation feeding the raw error
text into the output turns it red. Residual: the *keyword* classifier means hostile text
containing the word "timeout" lands in class `timeout`. That moves a candidate between
closed classes; it cannot inject text.

## Bounds

Reads are confined to `<root>/trajectories`. A symlink (file or directory) is never
followed out; a symlinked file is skipped and counted, a symlinked `trajectories`
directory that resolves outside the root is an error. Per scan: at most `max_files = 5000`
files (sorted by name; the rest are counted `over_file_cap`), at most
`max_file_bytes = 16 MiB` per file (the trajectory sink's own per-run cap; larger files
are `too_large`), at most `max_total_bytes = 256 MiB` in total (`over_byte_cap`). A file
that is not valid UTF-8 is `unreadable`; one with no valid JSON line is `corrupt`; single
bad lines are counted in `corrupt_lines` and the rest of the file is used. Nothing panics
on corrupt input.

## Observability

Events: `learn.candidates_extracted` (counts by kind), `learn.candidate_refused`
(`reason`, `count`), `learn.scan_completed` (files scanned, bytes read, duration) — printed
to stderr by `pii learn scan`. `pii metrics` shows `pii_learn_candidates{kind=…}` and
`pii_learn_refusals_total` once a scan has run. `pii health` lists the `learn` subsystem.

## Stage 2: verification (spec 124)

Extraction proposes; the verifier decides whether a proposal is **true**. `pii learn verify
<id>` or `pii learn verify --all` re-executes the graph a candidate is about with the REAL
engine, in a throwaway **shadow session**, and judges only from that run's own numbers. No
model is involved. Nothing acts on a verdict yet: stage 125 (promotion) is absent, and the
verifier changes no config, threshold or graph.

```sh
pii learn verify --all --traj <root> [--graphs <dir>] [--max-runs 5] [--run-ms 30000] \
    [--input '<json>'] [--scratch <dir>] [--protect <path>]...
```

**Isolation.** Each candidate gets a fresh directory under `--scratch` (default: the system
temp dir) holding its own store (`shadow.redb`), its own trajectory area and nothing else.
The run has an empty capability set, so it cannot write the configured workspace. The
verifier never opens the production store at all. If the scratch dir overlaps the learn
dir, the trajectory root, the graph dir or any `--protect` path, isolation cannot be
guaranteed and the verdict is `inconclusive (isolation)`; there is no best-effort run. The
shadow directory is removed when the verification ends. The test
`the_verifier_leaves_production_state_byte_identical` snapshots the production store (key
count and bytes, read from a copy because opening a redb file rewrites it), a workspace,
the graph dir and the trajectory root before and after a full pass, and asserts equality.

**How a claim is judged.** The candidate must be a `repeated_failure` or `stuck_loop` (the
claims a re-run can test; `limit_hit` and `cost_outlier` are `inconclusive (not_testable)`).
The graph is found from the cited trajectories and looked up by name among `<graphs>/*.toml`.
It is run as many times as the claim's own `threshold` (3 by default). The shadow
trajectories are then read by the **same extractor** that proposed the claim: the claim is
confirmed iff the same `dedupe_key` comes out again. Input is not stored in a trajectory
(only its hash), so every shadow run gets `--input` (default `null`); a graph whose failure
depends on its input needs it supplied, or the honest result is `refuted`.

**Three verdicts.**

* `confirmed`: the claim reproduced, with the shadow run's numbers (`shadow_runs`,
  `shadow_failed`, `threshold`).
* `refuted`: it did not reproduce. This is a **successful** verification: the verifier
  did its job and the lesson was false. A verifier that could only confirm would be no
  verifier, so the tests hand-write a false claim (diamond "fails" three times; it never
  does) and require `refuted`.
* `inconclusive` with a reason: `evidence_missing`, `evidence_unreadable`, `run_cap`,
  `timeout`, `not_testable`, `graph_unavailable`, `isolation`, `engine_error`.

**Absence of evidence is not evidence of absence.** If a cited trajectory no longer exists
(spec 115 retention pruned it) the verdict is `inconclusive (evidence_missing)`, naming the
missing paths. It is never `refuted` and never `confirmed`.

**Budgets** (both configurable): `max_shadow_runs = 5` per candidate (a claim whose
threshold needs more is `inconclusive (run_cap)` with `bound = max_shadow_runs=N`, before
anything runs) and `shadow_run_ms = 30000` per run (`inconclusive (timeout)` with the bound
and the elapsed time). A timed-out run's thread cannot be killed; it is abandoned and keeps
writing only inside its own shadow directory.

**Records.** Only verdicts are written: `<dir>/learn/verdicts/<candidate id>.json` (the
evidence used, missing paths, shadow session `s-<id>` and run ids, numbers, bound, elapsed)
and `<dir>/learn/verify_status.json`. A batch is recorded per candidate; nothing is
averaged. `pii learn show <id>` prints the verdict when one exists.

**Observability.** Events `learn.verification_started`, `learn.candidate_confirmed`,
`learn.candidate_refuted`, `learn.candidate_inconclusive` (each with the candidate id, the
numbers, and the reason and bound where relevant). `pii metrics` shows
`pii_learn_verdicts{verdict=...}` for the last pass, next to the extraction counters, and
the `learn` health entry reports `last_verification`.

## What was not verified

* Verification reproduces a *failure* claim by re-running the graph with an empty capability
  set and the supplied input; a graph that fails only under the original input, capabilities
  or external state may come back `refuted` or confirmed for a different reason than the
  original. The failure class is compared, not the cause.
* A timed-out shadow run is abandoned, not killed.
* The daemon never verifies; it only reads the verdict status. Its counters were checked
  against a live daemon once, by hand, not by an automated test.

## Stage 1 caveats (spec 123)

* No real trajectory this repo had contained a failure or a limit event, so a scan of the
  existing soak/gate trajectories yields zero candidates (correct, and verified). The real
  candidates in the spec report come from runs the real engine wrote for this purpose.
* Thresholds are defaults chosen by reasoning, not tuned on production data.
* The `learn` health group is read-only over `status.json`; it has no live daemon test.
