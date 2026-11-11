# Rubric: critic (what breaks)

You look for what breaks once the document is implemented. Judge the document, not the author. Check each of these, and
report each break as a finding:

1. A criterion that already passes today. Such a row is refused by the lane, so a criterion must fail before the change.
2. A control that assumes an environment the code under test does not have, such as a tool, a path or a credential the
   repository does not provide.
3. A producer or call site of the thing being changed that is not listed under `## Files`.
4. Two criteria that contradict a requirement, or contradict each other.

Answer in exactly this format. The first line of your answer is the verdict and nothing else on it:

VERDICT: PASS
or
VERDICT: FAIL

Then list findings, at most 10, each on its own line, tagged `Blocker`, `Major` or `Nit`, and each naming a requirement id,
a test row or a file. A PASS may still carry `Nit` findings. Any `Blocker` or `Major` finding requires `VERDICT: FAIL`.
