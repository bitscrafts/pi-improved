#!/usr/bin/env bash
# drain-rechecks-the-hold.test.sh - a tree hold taken WHILE the drain is running stops the drain before its next cycle.
#
# MEASURED 2026-11-06: drain.sh checked the tree hold once, at the top of a firing, and then looped up to 8 cycles. A person
# (or resume-row-atomic.sh) who took the hold after the firing began was ignored: one firing re-dispatched the same row every
# ~3 minutes for ~25 minutes and three resumes failed. The fix is a re-check before every cycle. "cron must always be on"
# (operator, 2026-11-04) is why the answer is a yield, never a pause.
#
# The REAL drain.sh runs, in a throwaway git repo (PII_DRAIN_ROOT) that holds a copy of the live scripts/autonomy and a STUB
# cycle.sh. The stub is the only thing replaced: it records each call and, in the cases that need it, takes the hold the way
# an interactive turn does (hold.sh take). Nothing here touches the live tree, queue, hold, lock or control file.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
SRC=$(pwd)
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok   $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T=$(mktemp -d "$SRC/_tmp/drain-rechecks-hold.XXXXXX") || exit 2
trap 'find "$T" -depth -delete 2>/dev/null' EXIT

# ---- the throwaway repo: a copy of the real scripts, a queue with pending rows, a RUNNING control file ----
R=$T/repo; mkdir -p "$R/_tmp/autonomy" "$R/specs"
cp -r "$SRC/scripts" "$R/scripts"
# byte-compiled caches are not part of the scripts and Python REWRITES them on its first run, which would turn the scratch tree
# dirty and make the drain (rightly) refuse to dispatch - so they are neither copied nor tracked
find "$R/scripts" -depth \( -name tests -o -name __pycache__ \) -type d -exec rm -rf {} + 2>/dev/null
printf '_tmp/\n__pycache__/\n' > "$R/.gitignore"
printf 'RUNNING since test\n' > "$R/_tmp/autonomy/control"
printf '%s\n' '# scratch spec' '## Files' '- `scripts/autonomy/state.py`' '## Exit Criteria' '- [ ] `true`' > "$R/specs/900-scratch.md"
PL=$T/plan.tsv; ST=$T/state.json
for i in 1 2 3; do printf '%s\t900-scratch.md\t-\ttitle %s\ttrue\tscripts/autonomy/state.py\t1\n' "$i" "$i"; done > "$PL"
printf '{}' > "$ST"
( cd "$R" && git init -q && git add -A && git -c user.name=t -c user.email=t@t commit -q -m scratch ) || exit 2

# pgrep is stubbed (a live cron cycle on this machine must not make the drain yield for the wrong reason)
mkdir -p "$T/bin"; printf '#!/usr/bin/env bash\nexit 1\n' > "$T/bin/pgrep"; chmod +x "$T/bin/pgrep"
export PATH="$T/bin:$PATH"
export PII_HOLD=$T/hold PII_STATE=$ST PII_PLAN=$PL PII_SPECS=$R/specs PII_DRAIN_ROOT=$R PII_ROOT=$R
export PII_CRON=1 PII_DRAIN_QUIET=0 MAX_CYCLES=3 PII_LOCK_WAIT=5 CALLS=$T/calls

# The stub cycle is written and COMMITTED once (a tracked file that changes would make the tree dirty, and the drain rightly
# refuses a dirty tree). Its behaviour on the FIRST call is read from an untracked mode file: take | take-release | none.
MODE=$R/_tmp/mode
cat > "$R/scripts/autonomy/cycle.sh" <<EOF
#!/usr/bin/env bash
echo x >> "$CALLS"
N=\$(wc -l < "$CALLS")
if [ "\$N" = 1 ]; then
  case "\$(cat "$MODE" 2>/dev/null)" in
    take)         bash scripts/autonomy/hold.sh take mid-drain-person 600 100 >/dev/null ;;
    take-release) bash scripts/autonomy/hold.sh take mid-drain-person 600 100 >/dev/null; bash scripts/autonomy/hold.sh release mid-drain-person >/dev/null ;;
  esac
fi
echo "OUTCOME=done ITEM=\$N"
EOF
( cd "$R" && git add -A && git -c user.name=t -c user.email=t@t commit -q -m stub-cycle ) || exit 2
stub(){ printf '%s' "$1" > "$MODE"; }
run(){ : > "$CALLS"; rm -f "$PII_HOLD"; ( cd "$R" && bash scripts/autonomy/drain.sh 2>&1 ); }
calls(){ wc -l < "$CALLS" | tr -d ' '; }

# 1. THE DEFECT: the hold is taken during cycle 1 -> no second cycle
stub take; OUT=$(run)
check "a hold taken during cycle 1 stops the drain: exactly 1 cycle ran" "$(calls)" "1"
printf '%s\n' "$OUT" | grep -q 'held' && check "the drain says it stopped because the tree is held" yes yes || check "the drain says it stopped because the tree is held" "no (out: $(printf '%s' "$OUT" | tail -2 | tr '\n' ' '))" yes
# 2. CONTROL: nobody takes the hold -> the drain still runs every cycle it is allowed
stub none; OUT=$(run)
[ "$(calls)" = 3 ] || printf '%s\n' "$OUT" | tail -6 | sed 's/^/  drain said: /' >&2
check "CONTROL no hold: all 3 cycles run" "$(calls)" "3"
# 3. CONTROL: a hold that was taken and RELEASED during cycle 1 is gone by cycle 2 -> the drain goes on
stub take-release; OUT=$(run)
check "CONTROL a hold released before the next cycle does not stop the drain" "$(calls)" "3"
# 4. CONTROL (the existing rule): a hold in place BEFORE the firing -> zero cycles
stub none; : > "$CALLS"; ( cd "$R" && bash scripts/autonomy/hold.sh take person-first 600 100 >/dev/null ); ( cd "$R" && bash scripts/autonomy/drain.sh >/dev/null 2>&1 )
check "CONTROL a hold present at the start: no cycle runs" "$(calls)" "0"
echo "----"; echo "$OK ok, $FAILS FAIL"; [ "$FAILS" = 0 ]
