# Spec: a document is read by reviewers who see nothing but the document, before it is acted on

## Overview
Every stall this week came from a spec that nobody read before a worker was paid. Row 399 was refused 252 times (a criterion already passed); once past
that, its file list (21 paths) missed five files the worker had to change and named none of the producers of `tool = "bash"`, so the gate failed 25 minutes in
and the list grew to 34. Row 401's spec called the limit on `discord.rs` (443 lines) "the effective-line limit of 450", but the gate step that failed it counts
PHYSICAL lines, so the spec named the wrong counter, and the file ended at 465. Spec 374 still carries a false criterion. A reader with no context, holding a
written bar, finds each of these in minutes.

This row adds a GENERIC reviewing capability, built only from existing node kinds. Nothing in it is about specs: WHAT is judged lives in a rubric FILE, and WHICH
rubrics run is the caller's input. The same graphs review a spec before queueing (rubrics shipped here), a design note, a PR description or a documentation page,
and the multi-pii deliberation (note 080) calls them with its own rubrics. Design source: `_docs/06-research/081-egm-evaluation-2026-11-09.md`. A reviewer that cannot
run is not a pass; an unparseable answer is a failure. This graph is ONE round; convergence over rounds is the next row.

Two measured limits of the graph language decide the shape, so they are decisions here and not choices left to the implementer:
- An expression cannot index an array (`$.input.x.0` resolves to nothing), and there is no iteration primitive (`graphs/COMPOSABILITY.md:110`). So the set of
  reviewers is NAMED SLOTS (`rubric_1` .. `rubric_4`), not an array, and `review` is a fixed fan-out of four pinned `call`s. A caller needing more than four calls
  `review` twice. The limit is stated, not hidden.
- An `agent` node's `system` is a literal read at build. So the rubric text travels in the agent's PROMPT (a `{{path}}` template, resolved at run), and the `system`
  literal is the same one-line instruction for every reviewer.

**Project**: <repo>
**Phase**: review before spend (follows 402; the deliberation design in note 080 reuses these graphs with its own rubrics)
**Depends on**: 399

**Measured 2026-11-11 (after rows 404 and 409 landed):** the fan-out loads and evaluates; 18 of the 19 `review_graph` tests pass with the parked work in place. The one that fails is
`review_with_one_failing_slot_names_that_rubric_only`: the run is refused, as it must be, but the refusal reads `a requested rubric did not pass, or rubric_1 was not given; each slot's verdict
is in the journal` - it does not name the rubric that failed, and the test (line ~515) requires `rubric 2` in the message. The text is yours (`graphs/review.toml`). The engine now supports it (row 410, `f736d84`): a `refuse` node's `reason` may hold `{{path}}` placeholders, rendered at run time from the node's own context
with the same renderer a prompt uses; a placeholder that resolves to nothing leaves the reason as written. Build the refusal's reason so it names exactly the rubrics that did not pass, and no other;
each slot's rubric name is a run input (`rubric_1` .. `rubric_4`). Do not weaken the test; do not touch the engine (`crates/pii-graph`).

## Requirements
- R1 `graphs/review-one.toml` (entry `rubric`, exit `done`). Input: `doc` (repo-relative path), `rubric` (a name), `workdir`, `model` (a role such as `@planner`), `provider`.
  Nodes: `rubric` (a `tool` node, `tool = "run"`, `programs = ["cat"]`, argv `["cat", "rubrics/" + name + ".md"]`, cwd `workdir`, so the text is the file's bytes),
  `reviewer` (an `agent` node), `verdict` (`transform`/`route` nodes), `done`, and a `refuse` arm. No absolute path, no credential, no model name, no rubric text.
- R2 A rubric name is refused, before anything is read, when it is empty, contains `/` or `..`, or starts with `.` (built with `contains`/`starts_with`/`length`).
  A missing rubric file is a failure naming the rubric. The bar is data: adding a reviewer is adding `rubrics/<name>.md`, never editing a graph or the binary.
- R3 The `reviewer` agent is read-only by construction: `tools = ["fs_read"]`, `caps = ["fs_read"]`, no `memory`, no `net`, no `proc`, `max_turns` at most 12, `system`
  the literal "You are a read-only reviewer. Judge only the document named below against the rubric below. Answer in the format the rubric requires.", and a
  `prompt` template that carries the rubric text (`{{$.nodes.rubric.stdout}}`) and the document path. It sees the document and what it opens with `fs_read`:
  no run history, no other reviewer's text, no queue state.
- R4 Verdict parsing is fail-closed and uses the existing text functions of spec 392 (`lower`, `trim`, `starts_with`, `contains`). Every rubric requires the answer's
  FIRST line to be exactly `VERDICT: PASS` or `VERDICT: FAIL`. A reviewer passes only when the answer, trimmed and lower-cased, is exactly `verdict: pass` or begins with
  `verdict: pass` followed by a newline. `VERDICT: FAIL`, `VERDICT: PASSING`, an empty answer, prose ("this does not pass", "looks good"), a node error, a refused rubric
  name, a missing rubric and a reviewer that did not run are ALL failures. On failure the run is refused and the refusal text is the reviewer's findings.
- R5 `graphs/review.toml` (entry `fan`, exit `done`). Input: `doc`, `workdir`, `model`, `provider`, and `rubric_1` .. `rubric_4` (at least `rubric_1`). Four `call` nodes pin
  `review-one` by sha256 (`slot_1` .. `slot_4`); a slot whose `rubric_N` is absent is `skipped` by a `route` and is neither a pass nor a failure. Port `ok` only when every
  requested slot passed. Otherwise the run ends refused, and the refusal names each failing rubric with its findings. A passing reviewer's text is not echoed.
  An absent `rubric_1` is refused (a review with no reviewer is not a pass).
- R6 The shipped rubrics, each ending with the R4 format requirement and "at most 10 findings, each tagged `Blocker`, `Major` or `Nit`, naming a requirement id, a test row or a file":
  - `rubrics/readiness.md`: the zero-decision rule. A competent engineer who does not know the repo must implement the document correctly on the first attempt without
    guessing. Every file to create, modify or delete is listed under `## Files`; every step names what changes and where; every requirement has a test row; every criterion
    is a command whose output the document predicts; no adjacent refactor is allowed.
  - `rubrics/critic.md`: what breaks. A criterion that already passes today (the lane refuses such a row); a control that assumes an environment the code under test does
    not have; a producer or call site of the thing being changed that is not listed; two criteria that contradict a requirement.
  - `rubrics/gate-parity.md`: for each file under `## Files`, open `gate-steps.txt` and `_docs/05-operations/crate-audit-baseline.tsv`, say which gate step judges it and
    which line counter it uses, and its current line count. FAIL when a file within 30 lines of 450 is listed and the document does not say where new code goes.
- R7 The corpus is files under `crates/pii-server/tests/fixtures/review/` (see below). `scripts/measure-review.sh <rubric,rubric,...> <dir>` runs the live reviewers over every
  `.md` in a directory and prints one row per document with the verdicts. Corpus convention: `bad-*.md` must FAIL and `ok-*.md` must PASS; the script exits non-zero otherwise.
  It is a measurement, not a criterion (it needs a model), and it takes the directory as an argument so any corpus works.

## Architecture
```
review-one {doc, rubric, workdir, model, provider}
  name check ─► rubric = tool run cat rubrics/<name>.md ─► reviewer = agent(fs_read only, prompt = rubric text + doc path)
              ─► verdict (first line exactly "verdict: pass"?) ─ yes ─► done
                                                              └─ any other ─► refuse(findings)
review {doc, workdir, model, provider, rubric_1..rubric_4}
  slot_1..slot_4 = call review-one (pinned); absent rubric_N -> skipped ─► join(all requested passed) ─ok─► done
                                                                                       └─ else ─► refuse(names each failing rubric)
```

## TDD Contract
All tests use the scripted provider (no model call). Each refusal has its near-miss.
| Test Name | Input | Expected Output |
|---|---|---|
| `a_reviewer_that_answers_pass_passes` | `review-one`, answer `VERDICT: PASS` | run `Succeeded`, port `ok` |
| `CONTROL_pass_with_findings_after_it_still_passes` | answer `VERDICT: PASS\n- Nit: wording` | passes |
| `prose_that_contains_pass_is_a_failure` | answer `this does not PASS` | refused |
| `a_verdict_that_only_starts_like_pass_is_a_failure` | answer `VERDICT: PASSING` | refused |
| `a_fail_verdict_refuses_with_its_findings` | answer `VERDICT: FAIL\n- Blocker R3: ...` | refused; text holds `Blocker R3` |
| `an_empty_answer_is_a_failure` | answer `` | refused |
| `a_reviewer_that_errors_is_a_failure` | provider errors | refused, never `ok` |
| `a_missing_rubric_is_a_failure_naming_it` | `rubric = "nope"` | refused naming `nope` |
| `a_rubric_name_that_leaves_the_directory_is_refused` | `"../x"`, `"a/b"`, `".hidden"`, `""` | each refused; nothing read outside `rubrics/` |
| `CONTROL_a_plain_rubric_name_is_accepted` | `"critic"` | the rubric is read |
| `the_rubric_text_reaches_the_agent_and_is_not_in_the_graph` | scripted provider records its prompt | prompt holds the rubric file's bytes and the doc path and no other run input; `graphs/review-one.toml` holds no rubric text |
| `a_rubric_added_as_a_file_is_a_reviewer_with_no_other_change` | a new `rubrics/<tmp>.md` in a temp repo | it runs; both graph files byte-identical |
| `every_reviewer_is_read_only` | load `review-one`, run it | the agent node's tools and caps are a subset of `{fs_read}` |
| `CONTROL_the_read_only_check_catches_a_reviewer_with_bash` | the same structural check on a copy with `bash` added | the check FAILS |
| `review_with_all_slots_passing_ends_ok` | `rubric_1..3` set, all `PASS` | port `ok`; slot 4 `skipped` |
| `review_with_one_failing_slot_names_that_rubric_only` | slot 2 `FAIL` | refused naming rubric 2 and not the others |
| `CONTROL_a_skipped_slot_is_not_a_failure` | only `rubric_1` set, `PASS` | `ok` |
| `review_without_rubric_1_is_refused` | no `rubric_*` | refused, never `ok` |
| `the_graphs_load_under_the_vocabulary_fence` | `lint-graph-kinds` over the tree | 14 kinds, 0 specialised |

Acceptance tests (each is what a plausible implementation gets wrong while passing every other row):
- `prose_that_contains_pass_is_a_failure` — a `contains("pass")` check.
- `a_rubric_added_as_a_file_is_a_reviewer_with_no_other_change` — hardwiring three reviewers into the graph, which is exactly what "generic" rules out.
- `the_rubric_text_reaches_the_agent_and_is_not_in_the_graph` — pasting the three rubrics into `system` literals.

Corpus (R7), exact sources so nothing is chosen by the implementer; each fixture is a byte copy, and a test asserts it starts with the spec's own `# Spec:` heading:
- BAD `bad-399-files.md`: `git show 88e8746:specs/399-a-graphs-tool-node-runs-a-program-never-a-shell.md` (21 paths under `## Files`; the final spec lists 34; it names no producer
  of `tool = "bash"`: `grep -cE 'sop/compile|log_wiring'` is 0 here and 3 in the final spec).
- BAD `bad-401-size.md`: `git show 99990f1:specs/401-the-pii-channel-reads-a-forums-threads-and-answers-inside-them.md` (line 77 names the wrong counter, "effective-line limit", for a file at 443 lines).
- BAD `bad-374-stale.md`: `specs/374-a-graph-documents-the-code.md` at HEAD (criterion `graph = "docs"` is false since row 393).
- NEAR-MISS `ok-399-final.md`: `git show 131fc0f:specs/399-a-graphs-tool-node-runs-a-program-never-a-shell.md` (34 paths, three producers named).
- NEAR-MISS `ok-394.md`: `git show a4d29f2:specs/394-a-lock-checks-its-owner.md` (finished on the first attempt).
- NEAR-MISS `ok-398.md`: `git show bc51317:specs/398-the-lane-parity-goldens-stop-naming-a-pin-that-moved.md` (finished on the first attempt).

## Exit Criteria
### New in this row
Each is verified to FAIL before the change. A criterion that already passes is a Guardrail, never a criterion.
- [ ] `cargo test -q -p pii-server --test review_graph 2>&1 | grep -qE 'test result: ok. [1-9][0-9]* passed'`
## Guardrails
- Keep green (they pass once the artefacts exist, so they are not criteria: the lane writes them in its first step; measured 2026-11-10, a criterion
  that passes before the work refuses the row at the RED step): `test -f graphs/review-one.toml && test -f graphs/review.toml`; `test "$(ls crates/pii-server/tests/fixtures/review/*.md | wc -l)" -eq 6 && test "$(ls rubrics/*.md | wc -l)" -ge 3`; `test -x scripts/measure-review.sh && bash -n scripts/measure-review.sh`.
- Keep green (they pass today, so they are not criteria): `cargo run -q -p xtask -- lint-graph-kinds` and `cargo test -q -p pii-graph --test tool_surface_is_closed`.
- Do NOT add a node kind, an expression function or a Rust source file: everything here is graphs, rubric files, fixtures, one test file and one script.
- Do NOT give a reviewer `bash`, `fs_write`, `edit`, `net`, `memory` or any `proc` cap. The only `proc` is the `rubric` node's declared `["cat"]`.
- Do NOT make a verdict depend on a bare substring search for `pass`. Do NOT echo a passing reviewer's text.
- Do NOT put a model name, a rubric text or a reviewer list in a graph file: `model` and the `rubric_N` names are inputs, rubric text is a file.
- Do NOT name a graph, node or input after specs: a document is whatever the caller passes.
- Every dirty path must be listed under Files: list a file before you touch it.

## Files
- `graphs/review-one.toml`
- `graphs/review.toml`
- `graphs/README.md`
- `rubrics/readiness.md`
- `rubrics/critic.md`
- `rubrics/gate-parity.md`
- `crates/pii-server/tests/review_graph/main.rs`
- `crates/pii-server/tests/review_graph/recording.rs`
- `crates/pii-server/tests/review_graph/fan_out.rs`
- `rubrics/`
- `crates/pii-server/tests/fixtures/`
- `crates/pii-server/tests/fixtures/review/bad-399-files.md`
- `crates/pii-server/tests/fixtures/review/bad-401-size.md`
- `crates/pii-server/tests/fixtures/review/bad-374-stale.md`
- `crates/pii-server/tests/fixtures/review/ok-399-final.md`
- `crates/pii-server/tests/fixtures/review/ok-394.md`
- `crates/pii-server/tests/fixtures/review/ok-398.md`
- `scripts/measure-review.sh`
