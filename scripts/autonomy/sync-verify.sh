#!/usr/bin/env bash
# sync-verify.sh [--check] — an open row's verify column is a cache of its spec, never a hand copy (spec 327).
#
# Measured 2026-10-31: 76 of 183 rows could not be reconciled with their spec's Exit Criteria, 30 of them a
# dropped tail (A154 lost its `coder-demo.toml` grep outright). A row IS the gate: if its command list silently
# loses a criterion, nothing runs it and nothing notices.
#
# Two passes, because the gate runs in an export where the queue's state file does not exist:
#   · ALWAYS — every row names a spec that exists and has at least one `- [ ] \`cmd\`` criterion. This needs no
#     state and no git, so the same rule holds in the dev tree and in a fresh-gate export.
#   · WITH STATE — for a row that is still open (pending / in-progress / needs-review), column 5 must equal its
#     spec's criteria now. `--check` names each row whose stored text disagrees and writes nothing.
#   · FINISHED rows are FROZEN history: never rewritten, because regenerating a row from a spec amended later
#     would claim the work was held to criteria that did not exist when it ran. check-row-truth.sh judges those
#     against the spec at their own commit.
#
# Writes are atomic and touch only column 5; other columns and the row order are preserved byte for byte.
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2

CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

python3 - "$CHECK" <<'PY'
import importlib, os, sys

sys.path.insert(0, "scripts/autonomy")
state = importlib.import_module("state")
check = sys.argv[1] == "1"
plan_path = os.environ.get("PII_PLAN") or os.path.join(state.ROOT, "scripts", "autonomy", "plan.tsv")
state_path = os.environ.get("PII_STATE") or os.path.join(state.ROOT, "_tmp", "autonomy", "state.json")

st = state.load()
have_state = os.path.isfile(state_path)

broken, stale, narrative, frozen, ok = [], [], [], 0, 0
lines = open(plan_path).read().splitlines()
out = []
for line in lines:
    if not line or line.startswith("#"):
        out.append(line)
        continue
    f = line.split("\t")
    if len(f) < 7:
        out.append(line)
        continue
    status = st.get(f[0], {}).get("status", "pending")
    fresh = state.derived_verify(f[1])
    if fresh is None:
        # A missing spec is always broken: the row names a document that is not there. A spec whose criteria are
        # narrative rather than `- [ ] \`cmd\`` cannot be derived at all -- 16 rows from before the rule are like
        # that (A38/173 wrote its criteria as prose). They are REPORTED and, when they are finished rows, never
        # failed: the rule is that an OPEN row must be derivable, because that is the row a worker will be held to.
        if not os.path.isfile(os.path.join(state.SPECS, f[1])):
            broken.append(f'{f[0]} (spec {f[1]} missing)')
        elif have_state and status in state.OPEN:
            broken.append(f"{f[0]} (spec {f[1]} has no executable criteria, and the row is open)")
        else:
            narrative.append(f[0])
        out.append(line)
        continue
    if not have_state or status not in state.OPEN:
        frozen += 1
        out.append(line)
        continue
    if fresh == f[4]:
        ok += 1
        out.append(line)
    else:
        stale.append(f[0])
        f[4] = fresh
        out.append("\t".join(f))

for r in broken:
    print(f"SYNC-VERIFY: {r}: its spec has no readable Exit Criteria — a row that cannot be verified")
for r in narrative:
    print(f"SYNC-VERIFY: {r}: its spec's criteria are narrative, not executable items -- nothing to derive")
for r in stale:
    print(f"SYNC-VERIFY: {r}: the stored verify is not its spec's criteria" + (" (--check: not written)" if check else " — regenerated"))
if not check and stale:
    tmp = plan_path + f".tmp.{os.getpid()}"
    with open(tmp, "w") as fh:
        fh.write("\n".join(out) + "\n")
    os.replace(tmp, plan_path)
if not have_state:
    print("sync-verify: no queue state in this tree (an export), so openness is unknown; "
          f"every row checked only for a usable spec ({frozen} row(s))")
print(f"sync-verify: {ok} open row(s) in step, {len(stale)} stale, {frozen} finished/unjudged, "
      f"{len(narrative)} narrative-criteria (pre-rule), {len(broken)} broken")
sys.exit(1 if (broken or stale) else 0)
PY
