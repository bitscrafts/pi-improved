#!/usr/bin/env bash
# measure-proven-fix.sh <idx> <fingerprint> <closing-commit> <crate> <paths>
#
# The spending half of the pre-registered experiment (Opus judgement 061). For ONE candidate:
#   1. seed a scratch tree at <closing-commit>^ — a revision that really lacked the fix,
#   2. hand ONE worker only the incident evidence (no budget cap; a 1500 s ceiling per worker),
#   3. measure MECHANICALLY: a test the worker introduced must be RED in a copy with the base files restored and
#      GREEN in the worker's own tree. Nothing the worker reports is trusted.
# Records one row in _tmp/autonomy/experiment/results.tsv, with the cost MEASURED (not capped).
set -uo pipefail
. "$(dirname "$(readlink -f "$0")")/../autonomy/env.sh"
ROOT=/projects/pi-improved
cd "$ROOT" || exit 2
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/projects/_tmp/exp-target}   # one shared target: the first build pays

IDX="$1"; FP="$2"; CLOSED="$3"; CRATE="$4"; PATHS="$5"
BASE=$(git rev-parse --short "$CLOSED^") || exit 2
OUT=$ROOT/_tmp/autonomy/experiment
SCRATCH=/projects/_tmp/exp-prove-fix
mkdir -p "$OUT/briefs" "$SCRATCH"
TREE=$SCRATCH/$IDX-$FP
BRIEF=$OUT/briefs/$IDX-$FP.md
BASECOPY=$TREE.base
RESULT=$OUT/results.tsv
[ -f "$RESULT" ] || printf 'idx\tfingerprint\tclosed\tworker_rc\tturns\tcost_usd\tproof\tnote\n' >"$RESULT"

# seed (rule 0: an existing tree is moved aside, never deleted)
[ -e "$TREE" ] && mv "$TREE" "$TREE.old-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$TREE"
git archive "$BASE" | tar -x -C "$TREE"
( cd "$TREE" && find . -name '*.rs' -exec md5sum {} + 2>/dev/null | sort ) >"$OUT/manifest-$IDX.seed.md5"

cat >"$BRIEF" <<EOF
# An incident, handed to you as your only context

One of this loop's own failures, exactly as an incident record would have opened it:

- fingerprint: \`$FP\`
- what it was doing: the work of one spec, whose fix landed in \`$CLOSED\` (you are on the revision before it)
- where the evidence is: \`$ROOT/_tmp/autonomy/logs/\` (the loop's own OUTCOME lines) and, for a gate failure, the
  failing test's name in that cycle's gate output.

**What is asked.** Find the cause and leave a **named test** that fails on the revision you started from and passes
with your change. Work only inside \`$PATHS\`. Do not commit; a harness measures your tree. If the premise looks
false, say so and stop — a worker that refuses a false premise is doing its job.
EOF

echo "[$IDX] $FP — base $BASE, tree $TREE"

# ---- 1. the worker (no budget cap; a time ceiling so a runaway cannot hold the window) ----
( cd "$TREE" && timeout 1500 claude -p "$(cat "$BRIEF")" \
    --model sonnet --permission-mode bypassPermissions --max-turns 60 \
    --output-format json >"$OUT/worker-$IDX.json" 2>"$OUT/worker-$IDX.err" )
WRC=$?

read_json() { python3 -c 'import json,sys
try:
    d = json.load(open(sys.argv[1]))
    print(d.get(sys.argv[2], ""))
except Exception:
    print("")' "$1" "$2"; }
COST=$(read_json "$OUT/worker-$IDX.json" total_cost_usd)
TURNS=$(read_json "$OUT/worker-$IDX.json" num_turns)

# did the worker run at all?  (three distinct facts, never conflated)
if [ ! -s "$OUT/worker-$IDX.json" ]; then
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$IDX" "$FP" "$CLOSED" "$WRC" "$TURNS" "$COST" "unmeasured" \
    "the worker FAILED TO RUN (rc=$WRC, no result JSON) — this is not a measurement" >>"$RESULT"
  echo "[$IDX] the worker failed to run (rc=$WRC)"; exit 0
fi

# ---- 2. what changed: the seeded manifest versus the tree now ----
( cd "$TREE" && find . -name '*.rs' -exec md5sum {} + 2>/dev/null | sort ) >"$OUT/manifest-$IDX.now.md5"
if diff -q "$OUT/manifest-$IDX.seed.md5" "$OUT/manifest-$IDX.now.md5" >/dev/null 2>&1; then
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$IDX" "$FP" "$CLOSED" "$WRC" "$TURNS" "$COST" "unmeasured" \
    "the worker ran and left the tree byte-identical to the base" >>"$RESULT"
  echo "[$IDX] changed nothing (rc=$WRC turns=$TURNS cost=$COST)"; exit 0
fi

# ---- 3. the proof: a name the change introduced, RED on the base copy, GREEN in the worker's tree ----
[ -e "$BASECOPY" ] && mv "$BASECOPY" "$BASECOPY.old-$(date +%s)"
cp -a "$TREE" "$BASECOPY"
( cd "$BASECOPY" && git -C "$ROOT" archive "$BASE" | tar -x -C . )

# names in the diff: every `fn <name>(` line in a .rs file that the change touched
CHANGED_FILES=$(diff "$OUT/manifest-$IDX.seed.md5" "$OUT/manifest-$IDX.now.md5" | awk '/^[<>]/{print $3}' | sed 's|^\./||' | sort -u)
NAMES=$(for f in $CHANGED_FILES; do [ -f "$TREE/$f" ] && grep -ohE 'fn [a-z0-9_]{5,}\(' "$TREE/$f" 2>/dev/null; done | sed 's/^fn //; s/($//' | sort -u | head -30)

PROOF="unproven"; NOTE="no introduced test went red on the base and green on the fix"
for NAME in $NAMES; do
  ( cd "$BASECOPY" && timeout 900 cargo test -p "$CRATE" "$NAME" >/dev/null 2>&1 ); R_BASE=$?
  ( cd "$TREE"     && timeout 900 cargo test -p "$CRATE" "$NAME" >/dev/null 2>&1 ); R_FIX=$?
  if [ "$R_BASE" -ne 0 ] && [ "$R_FIX" -eq 0 ]; then
    PROOF="proven:$NAME"; NOTE="red on base (rc=$R_BASE), green on the fix (rc=$R_FIX)"; break
  fi
  NOTE="tried $NAME: base rc=$R_BASE, fix rc=$R_FIX"
done

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$IDX" "$FP" "$CLOSED" "$WRC" "$TURNS" "$COST" "$PROOF" "$NOTE" >>"$RESULT"
echo "[$IDX] worker rc=$WRC turns=$TURNS cost=$COST proof=$PROOF ($NOTE)"
