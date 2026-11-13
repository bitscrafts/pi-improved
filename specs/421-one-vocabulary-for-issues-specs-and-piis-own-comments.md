# Spec: one vocabulary for issues, specs and pii's own comments
#: depends=420 status=pending block=-

## Overview
Three artefacts describe the same thing and none of them reads the other. Measured 2026-11-13:

- `.github/ISSUE_TEMPLATE/task.yml` asks for *What changes*, *Exit criteria* and *Files it may touch* — the exact three things a spec needs — but nothing says they
  become the spec's `## Requirements`, `## Exit Criteria` and `## Files`, so the channel drafts a spec from the issue's free text instead of from those fields.
- There is no local spec template: 420 specs were written by copying the previous one, so the card line, the section names and the "a criterion must fail before
  the work" rule are re-derived every time.
- pii's own comments have no format: the graph posts `Received. Nothing runs until…`, `… is the spec drafted from this issue…` and
  `a spec could not be drafted from this issue`, and `scripts/report-row.sh` posts `pii row 419 done at 4dca4e1: …`. Three shapes, none machine-readable, so
  nothing can check a comment, dedupe one, or count what pii said on an issue. The one thing they share is that a person has to read them.

This row gives the three one vocabulary: the issue template says which spec section each field becomes, `specs/TEMPLATE.md` is the shape a spec takes, and every
comment pii writes starts with `pii: <kind>` — with room for the instance that publishes it (`pii@<instance>: <kind>`, spec 425) from the first day.

**Project**: <repo>
**Phase**: pii manages its specs through GitHub
**Depends on**: 420

## Requirements
- R1 `.github/ISSUE_TEMPLATE/task.yml` keeps its three required fields and says, in each field's description, which spec section it becomes: *What changes* →
  `## Overview` and `## Requirements`, *Exit criteria* → `## Exit Criteria`, *Files it may touch* → `## Files`. It gains one optional field, *Context*
  (links, what you tried), which the drafter may use as prose and which no section requires.
- R2 `specs/TEMPLATE.md` exists, is not numbered (the dispatcher resolves `specs/<N>-*.md` only), and carries: the card line `#: depends=- status=pending
  block=-`, and the sections `## Overview`, `## Requirements`, `## Architecture`, `## TDD Contract`, `## Exit Criteria`, `## Guardrails`, `## Files`. Its Exit
  Criteria section shows one criterion as a backticked shell command, and its text says a criterion must fail before the work and that `## Files` must equal the
  row's paths column.
- R3 Every comment pii writes matches `pii(@<instance>)?: (received|drafted|refused|row) ` at the start: the graph's three texts and `report-row.sh`'s. The
  instance form is accepted from day one but not required until a row introduces it (spec 425).
- R4 The wording stays readable on a phone and states the one thing that matters at that step: nothing runs until the owner has read the draft.
- R5 One check validates all of it — the issue template's fields and their mapping, the local template's sections and card line, and every pii-authored text in
  the tree — and it has a control that proves the validator can fail.

## Architecture
```
.github/ISSUE_TEMPLATE/task.yml   *What changes* / *Exit criteria* / *Files it may touch* / *Context*  -> the spec's sections
specs/TEMPLATE.md                 the shape a spec takes (card line + 7 sections + the two rules)
graphs/github-issue.toml          pii: received …   pii: drafted <spec> …   pii: refused …
scripts/report-row.sh             pii: row <row> <status> at <commit> — <spec>
scripts/checks/comment-and-template-vocabulary.sh   one validator, one --control
```
Key decision: the prefix carries the KIND, not the author's prose, so a comment can be counted and deduped without a model reading it. Rejected: a JSON block
hidden in the comment (unreadable on a phone, and the point of these comments is that a person reads them). Rejected: renaming the sections to match the issue
template's labels (the queue, the gate and 420 specs already read `## Exit Criteria` and `## Files`).

## TDD Contract
New `scripts/checks/comment-and-template-vocabulary.sh`, registered as a gate step. It reads the three artefacts and validates them against one pattern table;
`--control` runs the same validators over a scratch fixture that must be rejected.
| Test Name | Input | Expected Output |
|---|---|---|
| `the_issue_template_names_the_section_each_field_becomes` | task.yml | each of the three fields' description names its `##` section |
| `the_local_template_carries_the_sections_the_queue_reads` | specs/TEMPLATE.md | card line matches, and all seven sections are present |
| `the_local_template_is_not_numbered` | the same file | its name does not match `<N>-*.md` (the dispatcher must never resolve it) |
| `CONTROL_a_comment_without_the_prefix_is_rejected` | `--control` fixture with `Received. Nothing runs…` | the validator fails, and the check reports the fixture |
| `CONTROL_a_template_without_a_card_line_is_rejected` | `--control` fixture missing `#: depends=` | the validator fails |
| `every_pii_comment_in_the_tree_matches_the_vocabulary` | graph texts + report-row format | each matches `pii(@<instance>)?: (received\|drafted\|refused\|row) ` |

Acceptance: the check passes and `--control` fails; the graph's own test file (`crates/pii-server/tests/github_issue_graph.rs`) keeps its 10 assertions green with
the new texts, and `scripts/checks/report-row-comments-and-closes.sh` keeps its 7 green.

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `bash scripts/checks/comment-and-template-vocabulary.sh`

## Guardrails
- Keep green, unedited except for the texts themselves: `cargo test -q -p pii-server --test github_issue_graph` (10 assertions) and
  `bash scripts/checks/report-row-comments-and-closes.sh` (7).
- `specs/TEMPLATE.md` must never be resolvable as a row's spec: no leading number.
- Do NOT change the section names the queue reads, and do NOT add a section the checks do not know about.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `.github/ISSUE_TEMPLATE/task.yml`
- `specs/TEMPLATE.md`
- `graphs/github-issue.toml`
- `crates/pii-server/tests/github_issue_graph.rs`
- `scripts/report-row.sh`
- `scripts/checks/report-row-comments-and-closes.sh`
- `scripts/checks/comment-and-template-vocabulary.sh`
- `gate-steps.txt`
