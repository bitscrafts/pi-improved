#!/usr/bin/env bash
# precommit-refusal.test.sh — a commit the pre-commit build refused is offered again, keeps its phase marker, and reaches the phone (spec 316).
# Part A runs the REAL state.py pick against scratch state and plan files. Part B takes the case block of tick.sh from the file as text at
# run time and runs it with a stub say_once. Part C reads the precommit block's last= literals from cycle.sh and matches them against the
# SKIP_DISPATCH phase-marker glob read from the same file. Never runs a cycle, a worker, cargo or the network.
# Exits 0 only if every assertion holds. Scratch lives under _tmp/ of this repo and is removed by this script's own trap.
set -u
cd "$(dirname "$0")/../../.." || exit 1
PY="uv run --no-project python scripts/autonomy/state.py"
CYCLE=scripts/autonomy/cycle.sh
TICK=scripts/autonomy/tick.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
T=_tmp/precommit-refusal.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
ST=$T/state.json
PL=$T/plan.tsv
# a scratch spec, so the row judges (spec 283, spec 309) read a ## Files list and an ## Exit Criteria that match the rows
SPECS=$T/specs
mkdir -p "$SPECS"
printf '%s\n' '# scratch spec' '## Files' '- `scripts/autonomy/state.py`' '## Exit Criteria' '- [ ] `true`' > "$SPECS/900-scratch.md"
# row <id>: one plan row, seven tab-separated fields (id spec brief title verify paths gate)
row(){ printf '%s\t900-scratch.md\t-\ttitle %s\ttrue\tscripts/autonomy/state.py\t1\n' "$1" "$1"; }
# plan <id...>: writes the plan rows in the order given
plan(){ : >"$PL"; for i in "$@"; do row "$i" >>"$PL"; done; }
# state <json>: writes the state file
state(){ printf '%s' "$1" >"$ST"; }
# pick: what state.py pick prints on stdout for the current scratch files
pick(){ PII_STATE="$ST" PII_PLAN="$PL" PII_SPECS="$SPECS" $PY pick 2>/dev/null; }
# first <tsv>: the first field, the row id (or NONE)
first(){ printf '%s' "$1" | cut -f1; }
# field <tsv> <n>: field n of the TSV line
field(){ printf '%s' "$1" | awk -F'\t' -v n="$2" '{print $n}'; }

# ---- Part A: the real pick ----

# a-precommit-broken-row-is-offered: the row is printed with its session as field 9, rc 0
state '{"X1":{"status":"needs-review","last":"precommit-broken: error[E0425]: cannot find function","session":"S1"}}'
plan X1
OUT=$(pick); RC=$?
check "a-precommit-broken-row-is-offered" "$(first "$OUT")" "X1"
check "a-precommit-broken-row-is-offered (session is field 9)" "$(field "$OUT" 9)" "S1"
check "a-precommit-broken-row-is-offered (rc 0)" "$RC" "0"

# a-precommit-timeout-row-is-offered
state '{"X1":{"status":"needs-review","last":"precommit-timeout — the check was KILLED, not failed","session":"S1"}}'
plan X1
check "a-precommit-timeout-row-is-offered" "$(first "$(pick)")" "X1"

# CONTROL-no-session-is-not-resumable
state '{"X1":{"status":"needs-review","last":"precommit-broken: error[E0425]: cannot find function","session":""}}'
plan X1
check "CONTROL-no-session-is-not-resumable" "$(pick)" "NONE"

# CONTROL-a-parked-row-stays-parked
state '{"X1":{"status":"parked","last":"precommit-broken: error[E0425]: cannot find function","session":"S1"}}'
plan X1
check "CONTROL-a-parked-row-stays-parked" "$(pick)" "NONE"

# CONTROL-an-auth-failure-waits-for-a-person
state '{"X1":{"status":"needs-review","last":"lane-auth-failed","session":"S1"}}'
plan X1
check "CONTROL-an-auth-failure-waits-for-a-person" "$(pick)" "NONE"

# CONTROL-a-worker-rc-row-is-still-offered
state '{"X1":{"status":"needs-review","last":"worker-rc=1","session":"S1"}}'
plan X1
check "CONTROL-a-worker-rc-row-is-still-offered" "$(first "$(pick)")" "X1"

# CONTROL-a-window-closed-row-is-still-offered
state '{"X1":{"status":"needs-review","last":"window-closed","session":"S1"}}'
plan X1
check "CONTROL-a-window-closed-row-is-still-offered" "$(first "$(pick)")" "X1"

# the-row-is-not-blocked-behind-a-pending-one: X1 (refused commit) is picked before X2 (pending)
state '{"X1":{"status":"needs-review","last":"precommit-broken: error[E0425]: cannot find function","session":"S1"}}'
plan X1 X2
check "the-row-is-not-blocked-behind-a-pending-one" "$(first "$(pick)")" "X1"

# ---- Part B: the live case block of tick.sh, with a recording say_once ----

# the live case block: from the case line on OUT to its closing esac
BLOCK=$(sed -n '/^case "\$OUT" in/,/^esac/p' "$TICK")
# run <OUT> <ITEM> <NEXT>: runs the live block; prints KEY and LINE records
run(){
  (
    OUT=$1; ITEM=$2; NEXT=$3
    LASTLOG=_tmp/autonomy/logs/cycle-1.log; HEAD=abc1234
    say_once(){ local k=$1; shift; printf 'KEY %s\n' "$k"; printf 'LINE %s\n' "$@"; }
    eval "$BLOCK"
  )
}

# (0) the block is the live one, with the precommit arm and every outcome it had
for w in esac done verify-failed gate-failed worker-rc- dirty-tree bad-plan window-closed precommit-; do
  check "the-live-block-has-the-precommit-arm ($w)" "$(grep -cF -- "$w" <<<"$BLOCK" | grep -qv '^0$' && echo yes || echo no)" "yes"
done

# (b) a refusal the next cycle will retry says so, and is keyed as such
R=$(run precommit-broken A1 A1)
check "the-refusal-that-will-be-retried-says-re-gating (key)" "$(grep '^KEY' <<<"$R")" "KEY regate:A1:precommit-broken"
check "the-refusal-that-will-be-retried-says-re-gating (line)" "$(grep -c 're-gating with its session' <<<"$R" | grep -q '^1$' && echo yes || echo no)" "yes"

# CONTROL-a-refusal-nobody-can-resume-asks-for-a-hand
N=$(run precommit-broken A1 NONE)
check "CONTROL-a-refusal-nobody-can-resume-asks-for-a-hand (key)" "$(grep '^KEY' <<<"$N")" "KEY bad:A1:precommit-broken"
check "CONTROL-a-refusal-nobody-can-resume-asks-for-a-hand (line)" "$(grep -c 'needs a hand' <<<"$N" | grep -q '^1$' && echo yes || echo no)" "yes"

# CONTROL-the-two-keys-differ
check "CONTROL-the-two-keys-differ" "$([ "$(grep '^KEY' <<<"$R")" != "$(grep '^KEY' <<<"$N")" ] && echo yes || echo no)" "yes"

# ---- Part C: the precommit literals of cycle.sh against the live phase-marker glob ----

# the glob: the patterns of the SKIP_DISPATCH case arm, read from cycle.sh (e.g. *verified*|*gated*|*worker-ok*)
GLOB=$(grep -E '^ *\*verified\*.*\) SKIP_DISPATCH=1;' "$CYCLE" | sed -E 's/^ *(.*)\) SKIP_DISPATCH=1;.*/\1/')
# marked <literal>: yes when the live glob matches the literal as a case pattern, no otherwise
marked(){ eval "case \$1 in $GLOB) echo yes ;; *) echo no ;; esac"; }
# the last= literals the precommit block writes, cut before the first $ (the variable part)
PCLITS=$(grep -o 'last=[^"]*precommit-[^"]*' "$CYCLE" | sed -E 's/^last=//; s/\$.*//')
ALL=yes
while IFS= read -r L; do [ "$(marked "$L")" = yes ] || ALL=no; done <<<"$PCLITS"
check "the-cycle-writes-a-marker-the-glob-matches (two literals, all marked)" \
  "$([ "$(grep -c . <<<"$PCLITS")" -eq 2 ] && [ "$ALL" = yes ] && echo yes || echo no)" "yes"

# CONTROL-the-unmarked-literal-the-spec-removes-does-not-match
check "CONTROL-the-unmarked-literal-the-spec-removes-does-not-match" \
  "$(marked 'precommit-broken: error[E0425] cannot find function')" "no"

# CONTROL-the-house-precedent-matches
check "CONTROL-the-house-precedent-matches" "$(marked 'worker-ok; gate-timeout after 3600s — the gate was KILLED')" "yes"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
