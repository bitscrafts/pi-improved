#!/usr/bin/env bash
# check-ids.sh — a new row's id IS its work's number (spec 326 R1).
#
# Measured 2026-10-31: all 183 rows carry an `A` prefix, the plan's own header never defines it, no `B` row has
# ever existed, and the letter therefore classifies nothing. The spec number is the one token every artefact
# already carries (row column 2, brief filename, worker argv, session filename, commit message), so it becomes
# the id for rows written from spec 326 on.
#
# Legacy rule: a row whose spec number is BELOW the boundary keeps its A-id — those rows are done, and renaming
# them would orphan every session id and memory key that names one. Its A-id is an alias, not an error.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

BOUNDARY="${PII_ID_BOUNDARY:-326}"

python3 - "$BOUNDARY" <<'PY'
import importlib, sys

sys.path.insert(0, "scripts/autonomy")
state = importlib.import_module("state")
boundary = int(sys.argv[1])

bad, legacy, canonical = [], 0, 0
for r in state.rows():
    num = r["spec"].split("-")[0]
    if not num.isdigit():
        bad.append(f'{r["id"]}: spec {r["spec"]!r} does not start with a number')
        continue
    n = int(num)
    if r["id"] == num:
        canonical += 1
    elif n < boundary:
        legacy += 1          # an alias from before the rule; sessions and memory keys point at it
    else:
        bad.append(f'{r["id"]}: spec {n} is at or past the boundary {boundary}, so its id must be "{num}"')

for b in bad:
    print("ROW-IDS FAIL: " + b)
print(f"row-ids: {canonical} canonical, {legacy} legacy alias, {len(bad)} violation(s)")
sys.exit(1 if bad else 0)
PY
