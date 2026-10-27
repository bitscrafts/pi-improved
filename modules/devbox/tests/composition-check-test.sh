#!/usr/bin/env bash
# composition-check-test.sh — proves composition-check.sh's own logic without a container or a daemon.
# It sources the script (the main flow does not run when sourced) and feeds recorded sample rows to its pure functions.
set -uo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT=$HERE/composition-check.sh
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "ok: $1"; }
bad() { FAIL=$((FAIL+1)); echo "not ok: $1"; }

[ -f "$SCRIPT" ] || { echo "missing $SCRIPT"; echo "PASS: 0  FAIL: 1"; exit 1; }
# shellcheck source=/dev/null
source "$SCRIPT"

T=$HERE/../../_tmp/cc-test.$$
mkdir -p "$T" || { echo "cannot make scratch $T"; echo "PASS: 0  FAIL: 1"; exit 1; }
trap '[ -n "$T" ] && rm -f -- "$T"/* && rmdir -- "$T"' EXIT

# 1. a row that passes prints PASS with its letter; a row that fails prints FAIL
row a "run succeeded" 0 > "$T/r1"
row b "one graph.call" 1 >> "$T/r1"
grep -qx "PASS a run succeeded" "$T/r1" && ok "row prints PASS for code 0" || bad "row prints PASS for code 0"
grep -qx "FAIL b one graph.call" "$T/r1" && ok "row prints FAIL for code 1" || bad "row prints FAIL for code 1"

# 2. every expected row present and passing gives a clean verdict
: > "$T/full"
for k in a b c d e f g; do row "$k" "criterion $k" 0 >> "$T/full"; done
[ "$(verdict_line "$T/full")" = "COMPOSITION-CHECK: 7 pass 0 fail" ] && ok "all rows pass: 7 pass 0 fail" || bad "all rows pass: 7 pass 0 fail"

# 3. control: one row missing from the sample counts as a failure
grep -v '^PASS e ' "$T/full" > "$T/missing"
v=$(verdict_line "$T/missing")
case "$v" in "COMPOSITION-CHECK: 6 pass 1 fail") ok "a missing row is a fail: $v" ;; *) bad "a missing row is a fail: got '$v'" ;; esac

# 4. a FAIL row is a failure even when a PASS row for the same letter also exists
{ cat "$T/full"; echo "FAIL c criterion c"; } > "$T/both"
case "$(verdict_line "$T/both")" in *" fail"*) [ "$(verdict_line "$T/both")" != "COMPOSITION-CHECK: 7 pass 0 fail" ] && ok "a FAIL row overrides a PASS row" || bad "a FAIL row overrides a PASS row" ;; *) bad "a FAIL row overrides a PASS row" ;; esac

# 5. the verdict's exit code: 0 only when there are no failures
verdict_ok "$T/full" && ok "verdict_ok is 0 for a clean sample" || bad "verdict_ok is 0 for a clean sample"
verdict_ok "$T/missing" && bad "verdict_ok is non-zero for a sample with a missing row" || ok "verdict_ok is non-zero for a sample with a missing row"

# 6. the budget rule: the second spawn's cap is smaller than the first's by the first's reported cost
budget_shrinks 3.00 0.42 2.58 && ok "budget shrinks by the reported cost" || bad "budget shrinks by the reported cost"
budget_shrinks 3.00 0.42 3.00 && bad "an unchanged budget is refused" || ok "an unchanged budget is refused"

# 7. the guard, against a RECORDED `ps -eo pid,ppid,args` table: the check's own ancestors are not workers, an unrelated worker is
REC=$T/ps.recorded
cat > "$REC" <<'PS'
  PID  PPID COMMAND
    1     0 /sbin/init
  500     1 /usr/local/bin/claude --session-id 02f8 --model claude-haiku-5-5 -p implement spec 287
  510   500 /bin/bash -c bash modules/devbox/composition-check.sh
  520   510 bash modules/devbox/composition-check.sh
  530   520 ps -eo pid,ppid,args
  540   520 grep -E (^| |/)(claude|pi)( |$)|run-batchC
  600     1 /usr/local/bin/claude --session-id 9a1c --model claude-haiku-5-5 -p queue worker
PS
[ -n "$(busy_workers "$(cat "$REC")" 520 510 500 1)" ] && ok "an unrelated worker in the table refuses (control)" || bad "an unrelated worker in the table refuses (control)"
busy_workers "$(cat "$REC")" 520 510 500 1 | grep -q '^ *600 ' && ok "the unrelated worker is the one named" || bad "the unrelated worker is the one named"
busy_workers "$(cat "$REC")" 520 510 500 1 | grep -qE '^ *(500|510|520) ' && bad "the check's own ancestor worker is excluded" || ok "the check's own ancestor worker is excluded"
grep -v '^ *600 ' "$REC" > "$T/ps.ancestors-only"
[ -z "$(busy_workers "$(cat "$T/ps.ancestors-only")" 520 510 500 1)" ] && ok "only ancestors and the check's own lines: not in flight" || bad "only ancestors and the check's own lines: not in flight"

# 8. the ancestor chain is read from /proc: it starts at the pid itself and reaches this shell's parent
ancestor_chain $$ > "$T/chain"
grep -qx "$PPID" "$T/chain" && ok "ancestor_chain walks the ppid chain from /proc" || bad "ancestor_chain walks the ppid chain from /proc"
last=$(tail -1 "$T/chain"); [ "$(sed 's/.*) //' "/proc/$last/stat" | awk '{print $2}')" = 0 ] && ok "ancestor_chain stops at a process with no parent" || bad "ancestor_chain stops at a process with no parent"

# 9. row (f): the daemon's revision equals HEAD's short sha; a -dirty suffix is allowed and printed; anything else is STALE-BINARY
revision_verdict 913d0871234 913d0871234 | grep -qx "PASS f daemon revision 913d0871234 is HEAD 913d0871234" && ok "a revision equal to HEAD passes" || bad "a revision equal to HEAD passes"
revision_verdict 913d0871234-dirty 913d0871234 | grep -qx "PASS f daemon revision 913d0871234-dirty is HEAD 913d0871234 (dirty tree)" && ok "a -dirty revision passes and prints the suffix" || bad "a -dirty revision passes and prints the suffix"
revision_verdict 7ca198a1574d 913d0871234 | grep -q "^FAIL f STALE-BINARY" && ok "a stale revision fails as STALE-BINARY" || bad "a stale revision fails as STALE-BINARY"
revision_verdict 913d08712345 913d0871234 | grep -q "^FAIL f STALE-BINARY" && ok "a revision that only starts with HEAD's sha is stale" || bad "a revision that only starts with HEAD's sha is stale"

echo "PASS: $PASS  FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
