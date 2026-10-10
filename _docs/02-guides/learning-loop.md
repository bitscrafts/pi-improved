# The learning loop, stage 1: extraction (spec 123)

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

## What was not verified

* No real trajectory this repo had contained a failure or a limit event, so a scan of the
  existing soak/gate trajectories yields zero candidates (correct, and verified). The real
  candidates in the spec report come from runs the real engine wrote for this purpose.
* Thresholds are defaults chosen by reasoning, not tuned on production data.
* The `learn` health group is read-only over `status.json`; it has no live daemon test.
