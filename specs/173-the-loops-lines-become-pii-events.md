# Spec: the loop's lines become pii events

## Overview
The user's words: "all activity lines should pass through observability! you need to know everything." Today the loop's
OUTCOME= lines, the workers' streams and the gate verdicts live in FILES. They are not in pii's own event store, so the
thing that observes pii is not observed by pii. A gate verdict, a park and a dispatch should be pii events like any other,
and one reader should see everything.

**Project**: <repo>

## Requirements
- The loop reports its own lifecycle to the daemon: cycle start, dispatch (with session id), verify result, gate verdict
  with the failing step named, park, and commit (with the revision). Use the existing event/channel surface; no new daemon.
- A single `pii` query shows the last N loop events beside the daemon's own, so one reader covers both.
- If the daemon is unreachable the loop must still run and must record that it could not report (fail-open, but visible).

## Exit Criteria
- [ ] one loop cycle produces a visible sequence of pii events (cycle, dispatch, verdict) when queried through the CLI
- [ ] the daemon being down does not stop the loop, and the un-reported cycle is named in the next cycle's line

## Guardrails
- Do NOT block the loop on the daemon; the work must continue and the gap must be visible.
- Do NOT commit; the loop's orchestrator reviews and commits.
