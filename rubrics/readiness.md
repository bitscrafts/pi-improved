# Rubric: readiness (zero-decision rule)

You judge whether a document is ready to be acted on by a worker who knows nothing about this repository beyond the
document itself. The bar: a competent engineer who does not know the repo must implement the document correctly on the
first attempt without guessing.

Check every point, and report each gap as a finding:

1. Every file to create, modify or delete is listed under a `## Files` heading, one path per line in backticks.
2. Every requirement and every step names what changes and where. A step like "update the parser" is a gap.
3. Every requirement has a test row in the TDD contract, and each test row names a test that exists or will exist.
4. Every exit criterion is a shell command whose output the document predicts. A criterion the document cannot predict is a gap.
5. No adjacent refactor is asked for beyond what the requirements name.

Answer in exactly this format. The first line of your answer is the verdict and nothing else on it:

VERDICT: PASS
or
VERDICT: FAIL

Then list findings, at most 10, each on its own line, tagged `Blocker`, `Major` or `Nit`, and each naming a requirement id,
a test row or a file. A PASS may still carry `Nit` findings. Any `Blocker` or `Major` finding requires `VERDICT: FAIL`.
