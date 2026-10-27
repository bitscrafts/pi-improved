#!/usr/bin/env bash
# Test: the cycle's quota requeue path prints its OUTCOME line on the normal (not dirty-tree) path (spec 300, F8). The requeue block is taken
# from cycle.sh at run time, as text, and run under `set -u` with stubs for state.py (python3) and say. RECOVER is unset, as it is on the
# normal path. Nothing else of cycle.sh runs, so no lock, probe, worker or state file is touched. Scratch lives under _tmp/ and is moved to _trash.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
REPO=$(pwd); PASS=0; FAIL=0
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/cycle-requeue-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; mv "$SCRATCH" "$REPO/_tmp/_trash/cycle-requeue-$(date +%s)"; }

# block <file>: the quota requeue block of cycle.sh, from its `if` to the line that prints requeued-quota, closed with its `fi`
block() { sed -n '/WLOG:-}" \] && grep -qiE/,/OUTCOME=requeued-quota/p' "$1"; echo '  fi'; }
# run_block <name> <block-text-file>: the block under set -u with stubs; its stdout+stderr in $SCRATCH/<name>.out, rc in $SCRATCH/<name>.rc
run_block() {
  local n=$1 b=$2
  printf '%s\n' 'set -u' 'unset RECOVER' "F=$SCRATCH/$n" "ITEM=A9 ATTEMPTS=1 NEWSID=sid-9 WRC=0 WLOG=$SCRATCH/worker.log" \
    'say() { echo "say: $*"; }' 'python3() { echo "state: $*" >> "$F/state"; }' > "$SCRATCH/$n.sh"
  mkdir -p "$SCRATCH/$n"; cat "$b" >> "$SCRATCH/$n.sh"
  echo 'echo "after the requeue block"' >> "$SCRATCH/$n.sh"
  bash "$SCRATCH/$n.sh" > "$SCRATCH/$n.out" 2>&1; echo $? > "$SCRATCH/$n.rc"
}

# the worker's log: the CLI's own refusal line, as window.sh and the requeue grep both read it
printf '%s\n' "You've hit your session limit · resets 8:20pm (UTC)" > "$SCRATCH/worker.log"
BLK="$SCRATCH/requeue.block"; block scripts/autonomy/cycle.sh > "$BLK"
# the real text: a requeue block that sets the outcome, its own exit 0
if [ -s "$BLK" ] && grep -q 'OUTCOME=requeued-quota' "$BLK"; then :; else echo "FAIL setup (no requeue block in cycle.sh)"; trash; exit 1; fi

# 1: on the normal path the requeue prints its outcome and exits 0
run_block normal "$BLK"
if grep -q '^OUTCOME=requeued-quota ITEM=A9 SESSION=' "$SCRATCH/normal.out" && [ "$(cat "$SCRATCH/normal.rc")" = 0 ] \
  && grep -q 'status=pending' "$SCRATCH/normal/state"; then
  ok "requeue_prints_its_outcome"
else bad "requeue_prints_its_outcome (rc [$(cat "$SCRATCH/normal.rc")], out: $(head -c 300 "$SCRATCH/normal.out"))"; fi

# CONTROL 2: the old expression, $RECOVER without a default, dies under set -u and prints no outcome
sed 's/SESSION=\${RECOVER:-}/SESSION=$RECOVER/' "$BLK" > "$SCRATCH/old.block"
run_block old "$SCRATCH/old.block"
if [ "$(cat "$SCRATCH/old.rc")" != 0 ] && ! grep -q 'OUTCOME=requeued-quota' "$SCRATCH/old.out" && grep -q 'RECOVER: unbound variable' "$SCRATCH/old.out"; then
  ok "CONTROL_the_old_expression_dies"
else bad "CONTROL_the_old_expression_dies (rc [$(cat "$SCRATCH/old.rc")])"; fi

trash
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
