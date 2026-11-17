# Spec: a row's verdict is its citations, not the worker's word
#: depends=430 status=pending block=-

## Overview
Research 084 (`_docs/06-research/084-aqua-ambient-quality-agent-2026-11-15.md`) records the one stage pii lacks: a
separate reader that judges work against its own instructions AND **discards its own findings when the citations do not
support them**. AQuA's line, quoted in that note: *"An insight is its citations, not a self-reported confidence score."*

Today pii's verdict is mechanical (`row-verify`: the criteria exit 0) plus the worker's own report. Measured
2026-11-15: a row can pass every criterion and still not do its job, because the criterion only asks whether a command
succeeds — not whether the change in front of it is the change the spec described.

## Requirements
R1. `graphs/row-verdict.toml` takes a row id and reads, from the repository only: the spec, the diff of the row's
    commit, and each criterion's captured output. It does **not** read the worker's report (blind judgement).
R2. For every criterion it answers `proven | unproven | vacuous`, each with a citation: `path:start-end` for a code
    claim, or the evidence file for a command claim.
R3. `vacuous` is for a criterion that already passed before the row's change (what `red-audit` measures) — a criterion
    that passes without the work is not proof of the work.
R4. A second pass **discards** every `unproven` finding whose citation does not resolve to real content in the diff or
    the evidence file, and reports what it discarded. Without this stage the reviewer is noise (research 084 §3).
R5. The verdict is data: JSON on the graph's `end` node with one entry per criterion, and the row's overall verdict is
    `proven` only when nothing is `unproven` and nothing is `vacuous`.
R6. A cost cap is configuration (max turns per run), enforced by the graph, never left to the model's judgement.
R7. The verdict never edits the spec and never commits: it reports, a human or the queue acts.

## Composition
R8. Inputs: `spec`, `commit`, `evidence_dir`, `max_turns`. Outputs: `verdict`, `findings[]` (criterion, status,
    citation), `discarded[]`, `reason`. A caller (the queue, a gate step) branches on `verdict`, never on prose.
R9. Pure where it can be: the discard rule (does the citation resolve?) is a transform with unit tests, so the
    judgement stage is exercised with fixtures and no model.

## TDD Contract
| Test Name | Input | Expected Output |
| --- | --- | --- |
| a_proven_criterion_carries_a_citation | finding with a real path:range | kept, verdict `proven` |
| an_unproven_finding_without_a_resolving_citation_is_discarded | finding citing a path not in the diff | discarded |
| control_a_resolving_citation_is_never_discarded | finding citing a path in the diff | kept |
| a_vacuous_criterion_is_not_proof | criterion whose red-audit shows it passed before | `vacuous` |
| the_verdict_is_unproven_when_one_criterion_is_unproven | two proven, one unproven | `unproven` |
| the_cap_is_enforced_before_the_model_runs | max_turns below the graph's minimum | refused, named reason |
| the_worker_report_is_never_an_input | report file present beside the evidence | not read |

## Exit Criteria
- [ ] `pii graph lint graphs/row-verdict.toml`
- [ ] `bash scripts/checks/row-verdict-cites.sh`
- [ ] `cargo test -q -p pii-server --test row_verdict 2>&1 | grep -qE 'test result: ok\. ([6-9]|1[0-9]) passed'`

## Files
- `graphs/row-verdict.toml`
- `scripts/checks/row-verdict-cites.sh`
- `crates/pii-server/tests/row_verdict.rs`
- `gate-steps.txt`

## Notes
The queue uses this after a green gate and before `report-row.sh`: `proven` -> report and close;
`unproven` -> the row goes to `needs-review` with the findings printed. It is never a reason to skip the gate — it is a
second, independent reader of the same artefact.

## Guardrails
- Do NOT let the reviewer see the worker's report or the commit message as evidence of the work.
- Do NOT allow the model's confidence to replace a citation.
- Do NOT weaken the mechanical gate because this exists.
