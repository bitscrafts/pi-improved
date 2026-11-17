# AQuA, and the sweep we can build from it: judging the cron runs that report success

Filed 2026-11-15. **Research note; the runnable part is `/root/.hermes/scripts/cron-quality-sweep.sh` and the design
is `/projects/docs/05-operations/cron-quality-sweep.md`.** Nothing in pii is changed by this note.

## 1. What AQuA is, verified

Google's **Ambient Quality Agent**, published on the Google Developers Blog on **2026-10-08** by **Dima Melnyk and
Elia Secchi** ("The Outer Loop, Insights First"), reported by InfoQ on **2026-11-13** (Leela Kumili). The code is not
a standalone repository: it ships inside **`google/adk-recipes`** at `core/python/ambient-quality-agent/`, as an Agent
Development Kit agent tied to Cloud Trace / Cloud Logging / BigQuery.

It runs beside the observed agent, **outside the request path**, and never writes back. Its target is stated in one
line that describes our own worst incidents:

> "silent quality regressions that conventional infrastructure monitoring can miss, such as agents ignoring user
> preferences or skipping required tool calls while returning successful responses"

**Five-stage sweep**: sample (≤1,000 sessions) → review against a nine-point checklist with the agent's instructions,
tools and an optional `goal.md` in view, writing structured `actual`/`expected` findings → cluster by **failure
mechanism** → **verify**: a separate model checks each cluster against up to three full transcripts and **discards**
the clusters the evidence does not support → track as `NEW` / `RECURRING` / auto-`RESOLVED` after 14 days unseen.

Root-cause analysis is a separate on-demand step against an **immutable deploy-time source snapshot**, citing
`<path>:<start>-<end>` that the server validates against that revision. It proposes an edit and never applies one.

**Their evaluation, with their own caveat that it was controlled replay, not production traffic**: 32 journeys,
1,583 spans, 5 sessions clean, 27 sessions → 42 findings → 9 clusters → verifier rejected 3 → 6 verified. After
prompt fixes and replay: seat-availability bypasses 15 → 2, dietary-preference drops 7 → 0, clean sessions 5 → 13.

The line worth quoting in any design review:

> "An insight is its citations, not a self-reported confidence score."

## 2. Why it is in pii's research tree

Three of pii's standing problems are the same problem AQuA solves, and one of them is a rule in this repository:

- **A check that passes while the live path is broken** is the workspace's rule 0b, written after three certification
  failures in one day.
- **A job reporting success while having done nothing** is the cron failure recorded on 2026-10-10: `state=completed`,
  `last_status=error`, four hours unnoticed because cron output is saved rather than delivered.
- **An alert list nobody reads** is the incident table today: 38 rows open, oldest 66 days, no transitions.

AQuA's contribution is not new detection — it is the **verifier** and the **lifecycle**: an insight pipeline that can
reject its own findings and close its own tickets. Those are the two stages we lack.

## 3. What transfers, and what does not

**Transfers** (all implementable over `~/.hermes/cron/executions.db`, `jobs.json`, `output/<job_id>/` and the gateway
logs, with no cloud dependency):

1. **Five-stage shape** — sample, review, cluster, verify, track.
2. **Verifier as a separate model that discards unsupported findings.** Without it, a mechanical sweep becomes noise.
3. **Citations over confidence** — a finding is an output path, a job id, a verbatim error string; never a model's
   assurance that it found something.
4. **A cheap lifecycle** — `NEW` / `RECURRING` / auto-`RESOLVED` after 14 days unseen.
5. **An explicit cost cap in configuration**, not in the model's judgement.

**Does not transfer**: the ADK/Google Cloud packaging; the nine-point checklist (ours has to be written against *our*
failure modes); and the assumption that telemetry contains full transcripts — ours has output markdown and logs, and
some of what we would want to read was never written down.

## 4. The local design in one paragraph

`cron-quality-sweep.sh` samples the executions table, applies five deterministic signals (scheduled-but-failing,
delivered-nowhere, completed-in-no-time, completed-with-no-record, incident-never-closed), clusters them by job and
failure type, and prints each finding with its evidence path. Stages 4 and 5 are designed in the operations doc and
deliberately not built.

Its first run is itself a case study in rule 0b. It reported 44 findings; two thirds were false positives, because
the "completed in under 200 ms" and "output under 200 bytes" signals fired on five healthy `no_agent` script pollers
whose correct result is exactly a fast exit and a one-line file. Restricting those signals to **agent** jobs — where a
model call cannot complete in 200 ms — left **32 findings, all real**. A check that flags the healthy path is worse
than no check, and the only way this one was caught was by running it against live state instead of trusting it.

## 5. Status

Research, plus a read-only script and a design doc. No cron job created: adding a job to a system whose incident list
is already full of dead jobs is how the problem gets worse. The one live defect the sweep found —
`iracema-site` publish failing because `worker-build` is missing — has a one-line fix outside this note.
