# Spec: the lane-parity goldens stop naming a pin that moved

## Overview
`crates/pii-cli/tests/lane_plan_parity.rs` runs the shell lane (`scripts/run-batchC-pii.sh`) in a scratch
clone and compares its stdout against goldens captured in `tests/golden/lane/`. All 21 tests in the target
now fail on one line:

```
left : LANE-GRAPH: spec
       ABORT: graphs/docs.toml digest 'b7f8eacdb79aa051070046e19ca1e6c0527fde72f0df0383b08c0c192e020980'
       is not the pin in graphs/spec.toml ('3124cca3cf…c43901'): nothing started
right: LANE-GRAPH: spec / AGENT / MODEL / PROVIDER / SPEC 309 / …      (the golden: no abort)
```

`3124cca3…` is the digest of `graphs/sdd.toml`; `b7f8eacd…` is the digest of `graphs/docs.toml`, which IS
what `graphs/spec.toml` pins for its `docs` call. So the running lane read the wrong pin — the FIRST
`sha256` in the file rather than the one belonging to the `docs` call — which is exactly the bug fixed in
the lane at `scripts/run-batchC-pii.sh:186` (`grep -A6 'graph = "docs"'`).

Measured 2026-11-05, and the measurement is what makes this a row rather than a patch:

- the repository's own check passes: `grep -A6 'graph = "docs"' graphs/spec.toml` yields `b7f8eacd…`;
- a scratch built by hand exactly as `Scratch::new` builds it (copy `specs/ graphs/ scripts/`, `git init`,
  commit) also yields `b7f8eacd…`;
- the gate's export (`<scratch>/fresh-gate-*/scripts/run-batchC-pii.sh`) contains the fixed line;
- the target nevertheless aborts with the sdd digest.

So the harness rewrites or invokes a copy of the lane that is not the file those three checks read — the
rewrite at `lane_plan_parity.rs:214-226` substitutes the `REPO=` and `C=` lines, and something in that path
still runs the older reading. Until this is measured inside the harness, do not "fix the golden".

**Project**: <repo>
**Phase**: test infrastructure

## Requirements
### Functional
- R1 every test in the target reaches a verdict in under 10 s, and the target as a whole under 60 s.
- R2 the lane copy the harness executes is byte-identical to `scripts/run-batchC-pii.sh` except for the
  `REPO=` and `C=` lines; a test asserts that, so a rewrite that silently changes behaviour is refused.
- R3 the pin the lane checks is the pin of the `docs` call, proven by a control: a clone whose `docs.toml`
  is edited but whose pin is repaired still starts, and one whose pin is NOT repaired refuses with the
  docs digest named.
- R4 the goldens are regenerated only after R2 and R3 pass, and the regenerated diff is reviewed line by
  line: a golden that blesses an abort is worse than a red test.

### Non-Functional
- R5 no golden is edited by hand to make a test pass.

## TDD Contract
| Test name | Input | Expected |
|---|---|---|
| `the_executed_lane_differs_from_the_tracked_one_only_in_REPO_and_C` | scratch clone | exactly two lines differ |
| `the_pin_is_the_docs_call_pin_not_the_first_sha256` | spec.toml with a methodology route | `b7f8eacd…` |
| `CONTROL_an_unrepaired_docs_edit_still_aborts` | docs.toml edited, pin not repaired | abort names the docs digest |

## Exit Criteria
- [ ] `timeout 120 cargo test -q -p pii-cli --test lane_plan_parity 2>&1 | grep -qE 'test result: ok\. [1-9][0-9]* passed'`

Why this section has fewer criteria than the requirements imply: `sdd`'s contract step runs every
criterion BEFORE any code exists and refuses a row if one already passes (measured 2026-11-05: eight
dispatches refused, none spent). A criterion that passes today is a regression guard, not a test of the
new behaviour, so the guards are stated under Guardrails and enforced by the gate.

## Guardrails
- The regression guards that used to be criteria: `grep -q 'graph = "docs"' scripts/run-batchC-pii.sh` and
  `python3 deploy/graph-closure.py graphs $(sed -n '/^spec$/,$p' graphs/minimal.txt | tr '\n' ' ')` must stay green.
- Do NOT regenerate the goldens to match an abort: the golden says the lane starts, and that is the
  behaviour a row depends on.
- Do NOT make the pin check optional, and do not move it behind a flag: it is what makes a reviewed graph
  the graph that runs.
- Do NOT weaken the HEAD comparison to "the working tree": the pin protects the reviewed revision.

## Files
- `crates/pii-cli/tests/lane_plan_parity.rs`
- `crates/pii-cli/tests/golden/lane/spec-lane/stdout.txt`
- `scripts/run-batchC-pii.sh`
