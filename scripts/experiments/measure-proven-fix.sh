#!/usr/bin/env bash
# measure-proven-fix.sh <idx> <fingerprint> <closing-commit> <crate> <paths>
# The spending half of the pre-registered experiment. For ONE candidate:
#   1. seed a scratch tree at <closing-commit>^ (a state that really failed this fingerprint),
#   2. hand ONE budgeted worker only the incident evidence,
#   3. measure MECHANICALLY: every test name the worker's change introduces must FAIL in a copy of the tree with
#      the base files restored, and PASS in the worker's tree. Nothing the worker reports is trusted.
# Records one row in _tmp/autonomy/experiment/results.tsv, and spends at most $1 (--max-budget-usd).
set -uo pipefail
. "$(dirname "$(readlink -f "$0")")/../autonomy/env.sh"
cd /projects/pi-improved || exit 2
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/projects/_tmp/exp-target}   # one shared target: the first build pays, the rest are incremental

IDX="$1"; FP="$2"; CLOSED="$3"; CRATE="$4"; PATHS="$5"
BASE=$(git rev-parse --short "$CLOSED^") || exit 2
ROOT=/projects/pi-improved
OUT=$ROOT/_tmp/autonomy/experiment; mkdir -p "$OUT"
TREE=/projects/_tmp/exp-prove-fix/$IDX-$FP
BRIEF=$OUT/briefs/$IDX-$FP.md
mkdir -p "$OUT/briefs"

[ -e "$TREE" ] && mv "$TREE" "$TREE.old-$(date -u +%Y%m%dT%H%M%SZ)"   # rule 0: move aside, never delete
mkdir -p "$TREE"; git archive "$BASE" | tar -x -C "$TREE"

cat >"$BRIEF" <<EOF
# An incident, handed to you as your only context

One of this loop's own failures, exactly as an incident record would have opened it:

- fingerprint: \`$FP\`
- what it was doing: the work of one spec, whose change landed in \`$CLOSED\` (you are on the revision before it)
- where the evidence is: \`/projects/pi-improved/_tmp/autonomy/logs/\` (the loop's own OUTCOME lines) and, for a gate
  failure, the failing test's name in that cycle's gate output.

**What is asked.** Find the cause and leave a **named test** that fails on the revision you started from and passes
with your change. Work only inside \`$PATHS\`. Do not commit; a harness measures your tree. If the premise looks
false, say so and stop — a worker that refuses a false premise is doing its job.
EOF

echo "[$IDX] $FP — base $BASE, tree $TREE"
( cd "$TREE" && timeout 1500 claude -p "$(cat "$BRIEF")" \
    --model sonnet --permission-mode bypassPermissions --max-turns 40 --max-budget-usd 1 \
    --output-format json >"$OUT/worker-$IDX.json" 2>"$OUT/worker-$IDX.err" )
WRC=$?
COST=$(python3 -c 'import json,sys
try: d=json.load(open(sys.argv[1])); print(d.get("total_cost_usd"))
except Exception: print("")' "$OUT/worker-$IDX.json" 2>/dev/null)
TURNS=$(python3 -c 'import json,sys
try: d=json.load(open(sys.argv[1])); print(d.get("num_turns"))
except Exception: print("")' "$OUT/worker-$IDX.json" 2>/dev/null)

# which test names did the change introduce? (added `fn name(` lines in files under the item's paths)
NAMES=$( (cd "$TREE" && grep -rhoE 'fn [a-z0-9_]{6,}\(' --include=*.rs . 2>/dev/null) | sed 's/fn //; s/($//' | sort -u | head -40 )
CHANGED=$(cd "$TREE" && find . -name '*.rs' -newer Cargo.toml 2>/dev/null | head -20)

# the base copy: the same tree with the base files restored, so the introduced tests must be RED there
BASECOPY=/projects/_tmp/exp-prove-fix/$IDX-$FP.base
[ -e "$BASECOPY" ] && mv "$BASECOPY" "$BASECOPY.old-$(date +%s)"
cp -a "$TREE" "$BASECOPY" 2>/dev/null
( cd "$BASECOPY" && git -C /projects/pi-improved archive "$BASE" | tar -x -C . ) 2>/dev/null

PROOF="unmeasured"; NOTE=""
CHANGED=$(cd "$TREE" && find . -name '*.rs' -exec md5sum {} + 2>/dev/null | sort | md5sum)
SEEDED=$(md5sum "$OUT/manifest-$IDX.base.md5" | cut -d' ' -f1)
if [ -n "$CHANGED" ]; then
  # try each introduced name; the proof is a name that fails on base and passes on the worker's tree
  while read -r NAME; do
    [ -n "$NAME" ] || continue
    ( cd "$BASECOPY" && timeout 900 cargo test -p "$CRATE" "$NAME" >/dev/null 2>&1 ); R_BASE=$?
    ( cd "$TREE"     && timeout 900 cargo test -p "$CRATE" "$NAME" >/dev/null 2>&1 ); R_FIX=$?
    if [ "$R_BASE" -ne 0 ] && [ "$R_FIX" -eq 0 ]; then PROOF="proven:$NAME"; NOTE="red-on-base rc=$R_BASE / green-on-fix rc=$R_FIX"; break; fi
  done < <(printf '%s\n' "$NAMES")
  [ "$PROOF" = unmeasured ] && NOTE="no introduced test could be run (or none went red-then-green)"
else
  if [ ! -s "$OUT/worker-$IDX.json" ]; then NOTE="the worker FAILED TO RUN (rc=$WRC, no result JSON) — this is not a measurement"; else NOTE="the worker ran and left no change in the tree"; fi
fi

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$IDX" "$FP" "$CLOSED" "$WRC" "$TURNS" "$COST" "$PROOF" "$NOTE" >>"$OUT/results.tsv"
echo "[$IDX] worker rc=$WRC turns=$TURNS cost=$COST proof=$PROOF ($NOTE)"
