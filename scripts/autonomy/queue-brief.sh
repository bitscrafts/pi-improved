#!/usr/bin/env bash
# queue-brief.sh — what the queue session DID, on one page, generated from state and commits.
#
# Run it when the queue reaches empty, or at the end of a session:
#     bash scripts/autonomy/queue-brief.sh              # to stdout
#     bash scripts/autonomy/queue-brief.sh > brief.md   # as a report
#
# It reads only what is recorded — _tmp/autonomy/state.json, the plan rows, and git — and it PRINTS the rows that are not
# done instead of summarising them away. A brief that hides a parked or skipped row is worse than no brief: it is how
# "queue empty" came to look like success while three things sat undispatchable.
set -uo pipefail
cd /projects/pi-improved || exit 1
ST=_tmp/autonomy/state.json
PLAN=scripts/autonomy/plan.tsv
SINCE=${1:-$(git log -1 --format=%cd --date=format:%Y-%m-%d 2>/dev/null)}

python3 - "$ST" "$PLAN" "$SINCE" <<'PY'
import collections
import json
import subprocess
import sys

state_p, plan_p, since = sys.argv[1], sys.argv[2], sys.argv[3]
state = json.load(open(state_p))

plan = {}
with open(plan_p) as fh:
    for line in fh:
        f = line.rstrip("\n").split("\t")
        if len(f) >= 3:
            plan[f[0]] = (f[1], f[3] if len(f) > 3 else "")


def st(k):
    return (state.get(k) or {}).get("status", "-")


by = collections.Counter(st(k) for k in plan)

print(f"# QUEUE BRIEF — {since} onward\n")
print(f"rows in the plan: {len(plan)}")
for s, n in sorted(by.items(), key=lambda kv: -kv[1]):
    print(f"  {s:<16} {n}")

done = [k for k in plan if st(k) == "done"]
notdone = [(k, st(k)) for k in plan if st(k) != "done"]

print(f"\n## Closed: {len(done)} rows")
print("## NOT done — kept visible, never summarised away")
if not notdone:
    print("  none — every row is done")
for k, s in sorted(notdone):
    note = str((state.get(k) or {}).get("last", ""))[:100]
    print(f"  {k}  [{s}]  {plan[k][0]}  {note}")

out = subprocess.run(["git", "log", "--oneline", f"--since={since}"], capture_output=True, text=True).stdout
commits = [l for l in out.splitlines() if l.strip()]
cnt = subprocess.run(["git", "rev-list", "--count", f"--since={since}", "HEAD"],
                     capture_output=True, text=True).stdout.strip()
print(f"\n## Commits: {cnt or '?'} since {since}")
if not commits:
    # A zero here must not read as "nothing happened": the since-based query can miss when commit dates and the clock
    # disagree. Show the newest commits regardless, and say why.
    print("  (since-query returned none — showing the newest commits regardless, because a brief that reports zero")
    print("   commits while work landed is worse than no brief)")
    commits = subprocess.run(["git", "log", "--oneline", "-12"],
                             capture_output=True, text=True).stdout.splitlines()
for l in commits[:20]:
    print("  " + l[:100])
if len(commits) > 20:
    print(f"  … and {len(commits) - 20} more")

print("\n## Control and queue state")
ctl = ""
try:
    ctl = subprocess.run(["bash", "scripts/autonomy/control.sh", "status"],
                          capture_output=True, text=True).stdout.splitlines()[0]
except Exception:
    pass
print("  " + (ctl or "control: unreadable"))
print(f"  rows not done: {len(notdone)}")
PY
