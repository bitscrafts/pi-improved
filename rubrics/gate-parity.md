# Rubric: gate parity (which counter judges each file)

For each file under `## Files`, do this before you judge:

1. Open `gate-steps.txt` and find the gate step that judges that file. Say which step it is.
2. Open `_docs/05-operations/crate-audit-baseline.tsv` and find the line counter that step uses (physical lines or
   effective lines) and the file's current line count under that counter.
3. Report, for each file, the step, the counter and the current count.

Then judge: FAIL when a file within 30 lines of its limit is listed under `## Files` and the document does not say where
the new code goes. FAIL also when the document states a limit or a count that names a different counter than the gate
step uses for that file.

Answer in exactly this format. The first line of your answer is the verdict and nothing else on it:

VERDICT: PASS
or
VERDICT: FAIL

Then list the per-file facts you found, then findings, at most 10, each on its own line, tagged `Blocker`, `Major` or
`Nit`, and each naming a requirement id, a test row or a file. Any `Blocker` or `Major` finding requires `VERDICT: FAIL`.
