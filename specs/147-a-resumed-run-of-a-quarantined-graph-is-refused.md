# Spec: a resumed run of a quarantined graph is refused

## Overview

Filed with spec 125 and still open in `_docs/05-operations/open-items.md` §9: the refusal path that stops a
**new** run of a quarantined graph does not stop a run that is **resumed**. A quarantined graph can therefore be
driven forward through the back door — the exact hole the quarantine exists to close.

**Project**: /projects/pi-improved
**Phase**: the gate's own robustness (pre-video backlog, specs 145–148)

## Requirements

### Functional

R1. Every path that starts or continues a run consults **the same** quarantine/refusal check. One function, called
    from the start path and the resume path. No second implementation of the check.
R2. A refused resume is recorded as an event, and the run reaches the same terminal state a refused start reaches —
    it does not stay in flight, and it is not deleted.
R3. The refusal carries the same identifying detail the start path's refusal carries (which graph, which promotion
    quarantined it), so the record answers "why was this refused" without reading code.
R4. If a recovery path exists that continues runs after a daemon restart, it uses the same check. A new way to
    continue a run must not be able to bypass it.

### Non-Functional

R5. No behaviour change for a run whose graph is not quarantined: the same tests that pass today pass unchanged.

## TDD Contract

| Test Name | Input | Expected Output |
|---|---|---|
| a_resumed_run_of_a_quarantined_graph_is_refused | a run created before the quarantine, resumed after it | refused, terminal, event recorded |
| the_refusal_names_the_promotion | the same | the event carries the quarantining promotion's id |
| a_healthy_resume_still_works | a run of a graph with no quarantine | resumes and continues as before |

## Exit Criteria

- [ ] `cargo test -p pii-server --lib` prints `test result: ok` and the new test is in the list
- [ ] `grep -rn 'quarantin' crates/pii-server/src --include=*.rs | grep -c 'fn '` shows the check reachable from one place
- [ ] `cargo xtask fresh-gate` on this spec's commit is `ALL PASS`

## Guardrails

- Do NOT add a second quarantine check; route through the existing one.
- Do NOT delete a refused run — leave it terminal with the reason.
- Do NOT relax the start path to make the paths "consistent"; the resume path is the one that is wrong.
