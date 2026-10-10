# The learning loop: extraction (123), verification (124), promotion and lifecycle (125)

> **What this is, stated plainly.** Extraction reads the trajectory files the daemon
> already writes and **proposes** *candidate lessons*, each citing the runs and files that
> justify it. It is deterministic and offline: **no model call, no network**. It does
> **not act** — no config change, no threshold move, no code mutation comes out of a
> candidate. Stage 124 verifies; stage 125 (below) is the only stage that acts, and the
> only thing it can do is **stop a broken graph from being started**.

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

## Stage 3: promotion and lifecycle (spec 125)

The loop closes here: **propose → verify → promote → measure → revert.** This is the only
place in `pii` where a learned conclusion changes behaviour by itself, so every rule below
exists to make that safe.

```
pii learn promote <candidate-id> --metric graph_failed_runs --window-secs N
                  [--kind quarantine_graph] [--ttl-secs N] [--actor NAME] [--dir D]
pii learn promotions [--sweep [--traj <root>]] [--dir D]
pii learn revert <promotion-id> [--actor NAME] [--dir D]
pii learn ledger [--tail N] [--dir D]
```

Every command that writes prints the ledger id (`L<n>`) of the record it wrote.

**The whitelist has exactly one kind: `quarantine_graph`.** Applying it makes the engine
refuse to start that graph with the reason `learn: graph <name> is quarantined by promotion
<id>` (the run is journaled as a failed start and a `run.start.refused` event carries the
text), until the promotion is reverted or expires. Why one kind: it is visible, reversible
and measurable, and a loop that may only *stop* something is the smallest action that is
still an action. **Adding a second kind requires its own spec.** The code makes that one
addition: `PromotionKind` is an enum, and every function that depends on the kind is a
`match` on it, so a new variant does not compile until it has a target, a rollback and an
effect, and its tests are written. There is no string dispatch.

**The rules, in the order they are checked:**

1. *Only `confirmed` verdicts.* The verdict is **re-read from `<dir>/learn/verdicts/`** at
   promotion time, never passed in. No verdict, `refuted` and `inconclusive` are each
   refused with that reason, counted, and change nothing.
2. *Measurable or refused.* A promotion must declare its metric (`graph_failed_runs`) and a
   window (`--window-secs`, > 0). Without them it is refused at apply time: an unmeasured
   change is a guess wearing a promotion's clothes.
3. *The ledger first.* A record (promotion id, kind, graph, candidate id and verdict, the
   evidence, the named actor, the state transition) is appended to
   `<dir>/learn/ledger.jsonl` **before** the promotion exists. If the ledger cannot be
   written, nothing is applied.
4. *Rollback is stored, not planned.* The previous state is written into the promotion
   record at apply time. A promotion without rollback data is never in effect and is never
   applied.
5. *TTL.* A promotion lasts `--ttl-secs` (default **86400**, 24 h). After that the graph is
   usable again immediately (the engine's check compares the clock); the daemon's sweep then
   records it as `expired`.
6. *Outcome and automatic reversal.* When the window closes the daemon evaluates the
   metric: failed runs of that graph in the window **after** the promotion vs. the window of
   the same length **before** it, read from the trajectories by file modification time. If
   `after < before` the promotion is kept (ledger `outcome_kept`); otherwise it is
   **reverted automatically** (`auto_reverted`), with both numbers in the ledger and the
   `learn.promotion_reverted` event. Note what this does and does not prove: after a
   quarantine the graph does not run, so `after` is 0 by construction; the test with teeth is
   `before` — a promotion on a graph that was not failing in the preceding window (0 → 0) is
   not an improvement and is taken back, and a quarantine that did not take effect shows as
   `after >= before`.
7. *Refusals by construction (R6).* A promotion may not touch secrets, the observability
   contract, the gate, capability scopes, or any file outside its own store area. The
   mechanism is two things: the whitelist (any other kind is refused by name:
   `learn: promotion kind edit_config is not whitelisted (only quarantine_graph)`), and a
   scope check on **every** write (`write_scoped`): a path outside `<dir>/learn/`, one with a
   `..` below it, or a symlink is refused, e.g. `learn: promotion write outside its own store
   area refused: <path>`. Both are tested.

**States.** `active`, `reverted` (by a person), `expired` (TTL), `auto_reverted` (window).
`pii learn promotions` shows each with its verdict, metric, before/after and the decision in
words. Only one promotion may be in effect per kind and graph.

**Who advances it.** The daemon runs a sweep every 5 s (expire, evaluate, auto-revert) and
the engine checks the quarantine before every graph start. `pii learn promotions --sweep`
runs the same pass once without a daemon. The CLI writes the same files, so a promotion made
while the daemon is up takes effect on the next run start.

**Observability.** Events `learn.promotion_applied`, `learn.promotion_reverted` (manual and
automatic; `state` says which, `before`/`after` when measured), `learn.promotion_expired`,
`learn.promotion_refused` (`reason`), all in `names::ALL`. `pii metrics` shows
`pii_learn_promotions{state=...}` (`active`, `reverted`, `expired`, `auto_reverted`,
`refused`) and the `learn` health entry lists `active_promotions`.

**The ledger.** There is no `pii ledger` command and no shared decision ledger a CLI can
write: the heal-attempt store is a redb file the daemon holds open. Promotions therefore use
an append-only JSONL ledger in the learn area (`pii learn ledger`), one line per decision,
refusals included.

**Honest scope.** The loop's only action today is **stopping a broken graph**. It does not
edit code, config or thresholds, and it cannot widen anything. A resumed (suspended) run of a
quarantined graph is not refused; only starts are.

## What was not verified

* Verification reproduces a *failure* claim by re-running the graph with an empty capability
  set and the supplied input; a graph that fails only under the original input, capabilities
  or external state may come back `refuted` or confirmed for a different reason than the
  original. The failure class is compared, not the cause.
* A timed-out shadow run is abandoned, not killed.
* Promotion (125): the outcome metric is failed trajectories by file mtime, so a deployment
  with trajectories off measures 0 → 0 and every promotion is reverted at its window (the
  safe direction). The sweep interval (5 s) is a constant, not a config key. A CLI revert
  racing the daemon's sweep on the same promotion is last-writer-wins on one JSON file.
* The daemon never verifies; it only reads the verdict status. Its counters were checked
  against a live daemon once, by hand, not by an automated test.

## Stage 1 caveats (spec 123)

* No real trajectory this repo had contained a failure or a limit event, so a scan of the
  existing soak/gate trajectories yields zero candidates (correct, and verified). The real
  candidates in the spec report come from runs the real engine wrote for this purpose.
* Thresholds are defaults chosen by reasoning, not tuned on production data.
* The `learn` health group is read-only over `status.json`; it has no live daemon test.
