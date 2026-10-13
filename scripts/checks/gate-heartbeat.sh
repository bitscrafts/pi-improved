#!/usr/bin/env bash
# Harness for spec 146: the gate beats, names a hung step, has one step list, and a heartbeat is no verdict.
# Usage: gate-heartbeat.sh   (prints a heartbeat block, then `gate-heartbeat: ALL OK`; exit 0 only if every case holds)
# Each case copies the REAL quality-gate.sh + scripts/gate-lib.sh into a scratch gate dir with a tiny
# gate-steps.txt, so the real runner is exercised without cargo. Scratch lives under /projects/_tmp.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
base=/projects/_tmp
mkdir -p "$base" || exit 2
scratch="$(mktemp -d "$base/gate-hb.XXXXXX")" || exit 2
fails=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fails=$((fails + 1)); }

# new_gate <dir> <steps-text>: a minimal gate tree holding the real runner and the given step list.
new_gate() {
  mkdir -p "$1/scripts"
  cp "$root/quality-gate.sh" "$1/quality-gate.sh"
  cp "$root/scripts/gate-lib.sh" "$1/scripts/gate-lib.sh"
  printf '%s\n' "$2" >"$1/gate-steps.txt"
}

# 1. a_long_step_emits_heartbeats (+ a passing gate stays ALL PASS)
g="$scratch/beat"
new_gate "$g" 'slow|echo begun; sleep 4'
out="$(QG_HEARTBEAT_SECS=1 bash "$g/quality-gate.sh" 2>&1)"
rc=$?
echo "$out" | grep '^HEARTBEAT' | head -3
beats="$(echo "$out" | grep -cE '^HEARTBEAT step=slow elapsed=[0-9]+s last: begun$')"
[ "$beats" -ge 1 ] && ok "a_long_step_emits_heartbeats ($beats beats)" || bad "a_long_step_emits_heartbeats"
[ "$rc" -eq 0 ] && echo "$out" | grep -q 'quality-gate: ALL PASS' && ok "a passing gate is ALL PASS" || bad "passing gate rc=$rc"

# 2. the_step_list_has_one_source: the same added step runs locally and in a git-archive export.
src="$scratch/src"
new_gate "$src" 'a|echo step-a'
(cd "$src" && git init -q && git add -A && git -c user.name=t -c user.email=t@t commit -q -m one) || bad "scratch repo"
echo 'b|echo step-b' >>"$src/gate-steps.txt"
(cd "$src" && git add -A && git -c user.name=t -c user.email=t@t commit -q -m two)
local_out="$(bash "$src/quality-gate.sh" 2>&1)"
exp="$scratch/export"
mkdir -p "$exp"
(cd "$src" && git archive HEAD | tar -x -C "$exp")
export_out="$(cd "$exp" && bash quality-gate.sh 2>&1)"
echo "$local_out" | grep -q '^PASS: b$' && echo "$export_out" | grep -q '^PASS: b$' \
  && ok "the_step_list_has_one_source" || bad "the_step_list_has_one_source"
# the real export path calls the gate and carries no step list of its own
grep -q 'bash quality-gate.sh' "$root/xtask/src/fresh_gate.rs" && ! grep -q 'cargo clippy' "$root/xtask/src/fresh_gate.rs" \
  && ok "fresh-gate calls quality-gate.sh, no second list" || bad "fresh-gate re-implements steps"

# 3. a_hung_step_is_named: stalled step is killed and named; a step killed by a signal is named too.
g="$scratch/hung"
new_gate "$g" 'hang|echo started; sleep 60'
out="$(QG_HEARTBEAT_SECS=1 QG_STALL_SECS=2 bash "$g/quality-gate.sh" 2>&1)"
rc=$?
echo "$out" | grep -E '^FAIL: hang' | head -2
[ "$rc" -ne 0 ] && echo "$out" | grep -E '^FAIL: hang.*HUNG.*since last heartbeat.*last line: started' >/dev/null \
  && ! echo "$out" | grep -q 'ALL PASS' && ok "a_hung_step_is_named (stall)" || bad "a_hung_step_is_named (stall) rc=$rc"
g="$scratch/killed"
new_gate "$g" 'die|echo before; kill -9 $$'
out="$(bash "$g/quality-gate.sh" 2>&1)"
rc=$?
echo "$out" | grep -E '^FAIL: die' | head -2
[ "$rc" -ne 0 ] && echo "$out" | grep -E '^FAIL: die.*killed by signal 9.*since last heartbeat.*last line: before' >/dev/null \
  && ok "a_hung_step_is_named (killed)" || bad "a_hung_step_is_named (killed) rc=$rc"

# 4. a_heartbeat_is_not_a_verdict
g="$scratch/verdict"
new_gate "$g" 'beats|echo working; sleep 3; exit 3'
out="$(QG_HEARTBEAT_SECS=1 bash "$g/quality-gate.sh" 2>&1)"
rc=$?
[ "$rc" -ne 0 ] && echo "$out" | grep -q '^HEARTBEAT step=beats' && echo "$out" | grep -q '^FAIL: beats' \
  && ! echo "$out" | grep -q 'ALL PASS' && ok "a_heartbeat_is_not_a_verdict" || bad "a_heartbeat_is_not_a_verdict rc=$rc"

echo "scratch kept at $scratch"
if [ "$fails" -eq 0 ]; then echo "gate-heartbeat: ALL OK"; exit 0; fi
echo "gate-heartbeat: $fails FAILED"
exit 1
