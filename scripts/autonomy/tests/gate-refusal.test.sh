#!/usr/bin/env bash
# gate-refusal.test.sh — a gate or criteria failure is offered again with its session, keeps its phase marker, and says it will be retried (spec 318).
# Part A runs the REAL state.py pick against scratch state and plan files. Part B takes the case block of tick.sh from the file as text at
# run time and runs it with a stub say_once. Part C reads the gate and verify literals of cycle.sh and matches them against the SKIP_DISPATCH
# phase-marker glob read from the same file. Never runs a cycle, a worker, cargo or the network.
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
T=_tmp/gate-refusal.$$
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

# a-gate-failed-row-is-offered: the gate branch's marker form (worker-ok; gate-failed: ...), session in field 9, rc 0
state '{"X1":{"status":"needs-review","last":"worker-ok; gate-failed: check-clippy","session":"S1"}}'
plan X1
OUT=$(pick); RC=$?
check "a-gate-failed-row-is-offered" "$(first "$OUT")" "X1"
check "a-gate-failed-row-is-offered (session is field 9)" "$(field "$OUT" 9)" "S1"
check "a-gate-failed-row-is-offered (rc 0)" "$RC" "0"

# a-bare-gate-failed-row-is-offered: a row the old gate branch wrote (bare gate-failed, as A181 was) is offered too
state '{"X1":{"status":"needs-review","last":"gate-failed","session":"S1"}}'
plan X1
check "a-bare-gate-failed-row-is-offered" "$(first "$(pick)")" "X1"

# a-verify-failed-row-is-offered
state '{"X1":{"status":"needs-review","last":"verify-failed","session":"S1"}}'
plan X1
check "a-verify-failed-row-is-offered" "$(first "$(pick)")" "X1"

# CONTROL-no-session-is-not-resumable
state '{"X1":{"status":"needs-review","last":"worker-ok; gate-failed: check-clippy","session":""}}'
plan X1
check "CONTROL-no-session-is-not-resumable" "$(pick)" "NONE"

# CONTROL-a-parked-row-stays-parked
state '{"X1":{"status":"parked","last":"gate-failed","session":"S1"}}'
plan X1
check "CONTROL-a-parked-row-stays-parked" "$(pick)" "NONE"

# CONTROL-a-dirty-tree-waits-for-a-person
state '{"X1":{"status":"needs-review","last":"dirty-tree","session":"S1"}}'
plan X1
check "CONTROL-a-dirty-tree-waits-for-a-person" "$(pick)" "NONE"

# CONTROL-an-unlisted-changes-row-waits-for-a-person
state '{"X1":{"status":"needs-review","last":"unlisted-changes: scripts/x.sh","session":"S1"}}'
plan X1
check "CONTROL-an-unlisted-changes-row-waits-for-a-person" "$(pick)" "NONE"

# CONTROL-a-worker-rc-row-is-still-offered
state '{"X1":{"status":"needs-review","last":"worker-rc=1","session":"S1"}}'
plan X1
check "CONTROL-a-worker-rc-row-is-still-offered" "$(first "$(pick)")" "X1"

# CONTROL-a-window-closed-row-is-still-offered
state '{"X1":{"status":"needs-review","last":"window-closed","session":"S1"}}'
plan X1
check "CONTROL-a-window-closed-row-is-still-offered" "$(first "$(pick)")" "X1"

# CONTROL-a-precommit-broken-row-is-still-offered
state '{"X1":{"status":"needs-review","last":"precommit-broken: error[E0425]: cannot find function","session":"S1"}}'
plan X1
check "CONTROL-a-precommit-broken-row-is-still-offered" "$(first "$(pick)")" "X1"

# the-row-is-not-blocked-behind-a-pending-one: X1 (gate failed) is picked before X2 (pending)
state '{"X1":{"status":"needs-review","last":"worker-ok; gate-failed: check-clippy","session":"S1"}}'
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

# (0) the block is the live one, with both arms and every outcome it had
for w in esac done verify-failed gate-failed worker-rc- dirty-tree bad-plan window-closed precommit-; do
  check "the-live-block-has-both-arms ($w)" "$(grep -cF -- "$w" <<<"$BLOCK" | grep -qv '^0$' && echo yes || echo no)" "yes"
done

# (a) a gate failure the queue's pick names is re-gated, not handed to a person
G=$(run gate-failed A1 A1)
check "a-gate-failure-that-will-be-re-gated-says-so (key)" "$(grep '^KEY' <<<"$G")" "KEY regate:A1:gate-failed"
check "a-gate-failure-that-will-be-re-gated-says-so (line)" "$(grep -c 're-gating with its session' <<<"$G" | grep -q '^1$' && echo yes || echo no)" "yes"

# (b) a failed criterion the queue's pick names is retried with its session
V=$(run verify-failed A1 A1)
check "a-failed-criterion-that-will-be-retried-says-so (key)" "$(grep '^KEY' <<<"$V")" "KEY retry:A1:verify-failed"
check "a-failed-criterion-that-will-be-retried-says-so (line)" "$(grep -c 're-implementing with its session' <<<"$V" | grep -q '^1$' && echo yes || echo no)" "yes"

# CONTROL-a-gate-failure-nobody-can-resume-asks-for-a-hand
N=$(run gate-failed A1 NONE)
check "CONTROL-a-gate-failure-nobody-can-resume-asks-for-a-hand (key)" "$(grep '^KEY' <<<"$N")" "KEY bad:A1:gate-failed"
check "CONTROL-a-gate-failure-nobody-can-resume-asks-for-a-hand (line)" "$(grep -c 'needs a hand' <<<"$N" | grep -q '^1$' && echo yes || echo no)" "yes"

# CONTROL-the-retry-keys-differ-from-the-park-keys
NV=$(run verify-failed A1 NONE)
check "CONTROL-the-retry-keys-differ-from-the-park-keys" "$(grep '^KEY' <<<"$V")" "KEY retry:A1:verify-failed"
check "CONTROL-the-retry-keys-differ-from-the-park-keys (park)" "$(grep '^KEY' <<<"$NV")" "KEY bad:A1:verify-failed"

# CONTROL-a-dirty-tree-still-asks-for-a-hand
D=$(run dirty-tree A1 A1)
check "CONTROL-a-dirty-tree-still-asks-for-a-hand" "$(grep '^KEY' <<<"$D")" "KEY bad:A1:dirty-tree"

# ---- Part C: the gate and verify literals of cycle.sh against the live phase-marker glob ----

# the glob: the patterns of the SKIP_DISPATCH case arm, read from cycle.sh (e.g. *verified*|*gated*|*worker-ok*)
GLOB=$(grep -E '^ *\*verified\*.*\) SKIP_DISPATCH=1;' "$CYCLE" | sed -E 's/^ *(.*)\) SKIP_DISPATCH=1;.*/\1/')
# marked <literal>: yes when the live glob matches the literal as a case pattern, no otherwise
marked(){ eval "case \$1 in $GLOB) echo yes ;; *) echo no ;; esac"; }
# the gate branch's last= literal, cut before the first $ (the variable part)
GLIT=$(grep -o 'last=worker-ok[^"]*gate-failed[^"]*' "$CYCLE" | sed -E 's/^last=//; s/\$.*//')
check "the-gate-branch-writes-a-marker-the-glob-matches" "$(marked "$GLIT")" "yes"

# CONTROL-the-verify-literal-does-not-match: the verify branch stays bare, so the glob must not mark it
VLIT=$(grep -o 'last=verify-failed"' "$CYCLE" | sed -E 's/^last=//; s/"$//')
check "CONTROL-the-verify-literal-does-not-match (literal is bare)" "$VLIT" "verify-failed"
check "CONTROL-the-verify-literal-does-not-match" "$(marked "$VLIT")" "no"

# CONTROL-the-house-precedent-matches
check "CONTROL-the-house-precedent-matches" "$(marked 'worker-ok; gate-timeout after 3600s — the gate was KILLED')" "yes"

# CONTROL-the-unmarked-literal-the-spec-removes-does-not-match
check "CONTROL-the-unmarked-literal-the-spec-removes-does-not-match" "$(marked 'gate-failed')" "no"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
