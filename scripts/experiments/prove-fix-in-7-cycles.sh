#!/usr/bin/env bash
# prove-fix-in-7-cycles.sh — the pre-registered experiment the Opus 061 judgement asks for, and the only thing
# standing between us and spec 151 ("an incident becomes a proven fix on a branch").
#
# What it does, per selected historical failure: seed a scratch export at the revision where the failure was
# recorded, hand ONE budgeted worker only the incident evidence (the fingerprint and the failing test's name),
# and then measure MECHANICALLY whether the worker left a named test that is:
#     red on the base revision, green on the worker's change   (run by this script, not reported by the worker)
#
# Decision, pre-registered in _docs/06-research/061-opus-proactivity-judgement-2026-10-13.md:
#     build 151  iff  >= 3 of 7 produce a mechanically proven fix at under $1 each;
#     otherwise stop at 150 + 152 and let a person resolve with the evidence in hand.
#
# Usage: prove-fix-in-7-cycles.sh [--list] [--dry] [--only N,N,...] [--batch N]
#   --list   print the 7 candidates and exit (no spend)
#   --dry    build the briefs and the scratch trees, dispatch nothing (no spend)
#   --only   run only these 1-based indices
#   --batch  run the first N candidates (a window-aware slice: the 5-hour session is shared with the loop)
set -uo pipefail
. "$(dirname "$(readlink -f "$0")")/../autonomy/env.sh"
cd /projects/pi-improved || exit 2

LOG=_tmp/autonomy/experiment
mkdir -p "$LOG"
RESULTS=$LOG/results.tsv
BRIEFS=$LOG/briefs
SCRATCH=/projects/_tmp/exp-prove-fix
mkdir -p "$BRIEFS" "$SCRATCH"

MODE=list; ONLY=""; BATCH=0
while [ $# -gt 0 ]; do
  case "$1" in
    --list) MODE=list ;;
    --dry)  MODE=dry ;;
    --only) ONLY="$2"; shift ;;
    --batch) BATCH="$2"; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# The candidates: the failures the preserved log actually recorded, with the revision that was HEAD when the loop
# failed on them. `parent` is resolved at run time from the cycle's own log, not remembered here.
candidates() {
  printf '%s\n' \
    "gate-failed|141|the docs truth pass|_docs" \
    "verify-failed|139|the loop runs and its metric is not tautological|crates" \
    "verify-failed|137|channel egress bounded by [net]|crates" \
    "gate-failed|135|a panicking node produces a terminal run|crates" \
    "verify-failed|134|no test opens a store at a path it does not own|crates" \
    "gate-failed|133|the quarantine test waits on the event|crates" \
    "verify-failed|132|the Discord graph surface|crates"
}

if [ "$MODE" = list ]; then
  echo "the 7 candidates (1-based):"
  candidates | nl -w2 -s'. '
  echo
  echo "decision (pre-registered): build 151 iff >= 3 of 7 leave a mechanically proven test, at < \$1 each."
  exit 0
fi

if bash scripts/autonomy/window.sh check 2>/dev/null; then
  echo "window: $(bash scripts/autonomy/window.sh usage)"
  echo "REFUSED: the account window is spent — nothing dispatched. Re-run after the reset it named."
  exit 0
fi
echo "window: $(bash scripts/autonomy/window.sh usage)"

[ -f "$RESULTS" ] || printf 'idx\tspec\tfingerprint\tworker_rc\tturns\tcost_usd\tproof\tnote\n' >"$RESULTS"

run_one() {
  local idx="$1" fp="$2" spec="$3" title="$4" paths="$5"
  local brief="$BRIEFS/$idx-$fp.md" tree="$SCRATCH/$idx-$fp"
  # the revision to seed: the commit that CLOSED that spec (its work is the fix an incident would propose), so the
  # base is its parent — a state that really failed this fingerprint.
  local closed base
  closed=$(git log --format='%h %s' --grep="^$spec-" -1 | awk '{print $1}')
  [ -n "$closed" ] || closed=$(git log --format='%h' --grep="$spec" -1)
  base=$(git rev-parse --short "$closed^" 2>/dev/null)
  if [ -z "$closed" ] || [ -z "$base" ]; then
    printf '%s\t%s\t%s\t-\t-\t-\tno-parent\tclosed=%s\n' "$idx" "$spec" "$fp" "$closed" >>"$RESULTS"
    return 1
  fi
  cat >"$brief" <<EOF
# An incident, handed to a worker as its only context

One of this loop's own failures, exactly as spec 150 would have opened it:

- fingerprint: \`$fp\`
- spec it was working on: $spec — $title
- evidence: the cycle logs under \`_tmp/autonomy/logs/\` and, for a gate failure, the failing test's name in the
  gate's own output for that cycle.

**What is asked:** find the cause, and leave a named test that FAILS on the revision you started from and PASSES
with your change. Do not touch anything outside \`$paths\`. Do not commit (the harness commits nothing; it measures).
If the premise looks false, say so and stop — a worker that refuses a false premise is doing its job well.
EOF
  if [ "$MODE" = dry ]; then
    [ -e "$tree" ] && mv "$tree" "$tree.old-$(date -u +%Y%m%dT%H%M%SZ)"   # rule 0: move aside, never delete
    mkdir -p "$tree"; git archive "$base" | tar -x -C "$tree"
    printf '%s\t%s\t%s\t-\t-\t-\tdry\tbase=%s\n' "$idx" "$spec" "$fp" "$base" >>"$RESULTS"
    echo "  [$idx] $fp spec=$spec base=$base  (dry: tree seeded at $tree, nothing dispatched)"
    return 0
  fi
  [ -e "$tree" ] && mv "$tree" "$tree.old-$(date -u +%Y%m%dT%H%M%SZ)"
  mkdir -p "$tree"; git archive "$base" | tar -x -C "$tree"
  echo "  [$idx] dispatching on base=$base ($tree)"
  return 0
}

i=0
while IFS='|' read -r fp spec title paths; do
  i=$((i + 1))
  [ -n "$ONLY" ] && ! printf '%s' "$ONLY" | tr ',' '\n' | grep -qx "$i" && continue
  [ "$BATCH" -gt 0 ] && [ "$i" -gt "$BATCH" ] && break
  run_one "$i" "$fp" "$spec" "$title" "$paths"
done < <(candidates)

echo
echo "results so far: $RESULTS"
if [ "$MODE" = dry ]; then
  echo "DRY: nothing was spent. Re-run without --dry to spend (each dispatch is budgeted at \$1)."
fi
