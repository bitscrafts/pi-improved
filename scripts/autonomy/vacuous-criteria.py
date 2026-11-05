#!/usr/bin/env python3
"""vacuous-criteria.py [row...] — list exit criteria that can pass while proving nothing.

The shape: `cargo test ... <name filter> ... | grep -qE 'test result: ok'` with no `[1-9]` count. A filter that matches
no test prints `running 0 tests` / `test result: ok. 0 passed`, so the criterion is green for a test that does not
exist (measured 2026-11-05 on spec 394: `a_dead_holder_is_reaped_immediately` was never written and its criterion passed).
Read-only. Exit 1 when any is found, so it can be a gate step.
"""
import re
import subprocess
import sys

rows = sys.argv[1:]
lines = subprocess.run(
    ["python3", "scripts/autonomy/state.py", "list"], capture_output=True, text=True, check=True
).stdout.splitlines()
specs = {}
for line in lines:
    f = line.split("\t")
    specs[f[0]] = f[1]
bad = 0
for row in rows or sorted(specs):
    spec = specs.get(row)
    if not spec:
        continue
    crit = subprocess.run(
        ["python3", "scripts/autonomy/row-verify.py", "criteria", f"specs/{spec}"], capture_output=True, text=True
    ).stdout.splitlines()
    for c in crit:
        runs_tests = re.search(r"cargo test", c)
        reads_result = re.search(r"grep -q?E? ?'test result: ok", c) or "test result: ok" in c
        counts = re.search(r"\[1-9\]", c)
        if runs_tests and reads_result and not counts:
            bad += 1
            print(f"VACUOUS row {row}: {c[:150]}")
print(f"{bad} vacuous criteria")
sys.exit(1 if bad else 0)
