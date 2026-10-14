#!/usr/bin/env bash
# Harness for spec 151: drives the loop's OWN entry point (scripts/autonomy/incident-fix.sh) on a scratch clone of this
# repo seeded at the parent of 9fc7f9f, with only the worker (and, where a case needs it, the gate) injected.
# Usage: incident-to-branch.sh [--control master-moved]   (last line: INCIDENT-BRANCH OK | CONTROL OK; exit 0 only then)
# --control master-moved: the worker commits on master; the master-tip check must catch it, and say so.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
ctl="${2:-}"; [ "${1:-}" = "--control" ] || ctl=""
FIX=9fc7f9f
base_sha="$(git -C "$root" rev-parse "$FIX^")" || { echo "no $FIX in this repo"; exit 2; }
tmp=/projects/_tmp; mkdir -p "$tmp" || exit 2
scratch="$(mktemp -d "$tmp/incident-branch.XXXXXX")" || exit 2
fails=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fails=$((fails + 1)); }
TEST='grep -q EVENT_POLLS crates/pii-server/src/daemon/mod.rs'
WORKER="git diff $base_sha $FIX | git apply"
GATE='test -z "$(git status --porcelain)" && git log -1 --format=%s | grep -q "^incident "'

# mk <name> <test>: a scratch clone with master at the parent of the fix, plus one dirty-tree incident
mk() {
  local d="$scratch/$1"
  git clone -q "$root" "$d/repo" 2>/dev/null || { mkdir -p "$d"; git clone -q "$root" "$d/repo"; } || return 1
  git -C "$d/repo" checkout -q master && git -C "$d/repo" reset -q --hard "$base_sha" || return 1
  mkdir -p "$d/state"
  python3 - "$d/state/incidents.jsonl" "$base_sha" "$2" <<'PY'
import json,sys
json.dump({"fingerprint":"dirty-tree","first_seen":"2026-10-11T15:00:00Z","opened_at":"2026-10-11T15:20:00Z","count":4,
 "evidence":"_tmp/autonomy/logs/cycle-x.log:12","source":"cycle","test":sys.argv[3],"base":sys.argv[2]},open(sys.argv[1],"w"))
open(sys.argv[1],"a").write("\n")
PY
}
# run <name> <extra env...>: the real entry point, output in $scratch/<name>/out
run() {
  local n="$1"; shift
  ( env PII_REPO="$scratch/$n/repo" PII_INCIDENT_STATE="$scratch/$n/state" PII_INCIDENTS="$scratch/$n/state/incidents.jsonl" \
      PII_INCIDENT_GATE="$GATE" "$@" bash "$root/scripts/autonomy/incident-fix.sh" dirty-tree ) >"$scratch/$n/out" 2>&1
  echo $? >"$scratch/$n/rc"
}
tip() { git -C "$scratch/$1/repo" rev-parse master; }
specs_hash() { (cd "$root" && find specs -type f | sort | xargs sha256sum | sha256sum | cut -c1-16); }

# --- the control: a master that moves must be refused, and the harness must say so
if [ "$ctl" = "master-moved" ]; then
  mk ctl "$TEST" || { echo "setup failed"; exit 2; }
  before="$(tip ctl)"
  run ctl PII_INCIDENT_MERGE=0 PII_INCIDENT_WORKER="git -C $scratch/ctl/repo commit -q --allow-empty -m moved && $WORKER"
  after="$(tip ctl)"; legs="$(grep '^legs:' "$scratch/ctl/out")"
  if [ "$before" != "$after" ]; then echo "master-moved detected: $before -> $after (refused)"; echo "legs seen: $legs"; echo "CONTROL OK"; exit 0; fi
  echo "CONTROL FAILED: master did not move, the check could not have failed"; exit 1
fi

# 1+2+3. the brief, the branch with master untouched, the two proof legs
mk fix "$TEST" || { echo "setup failed"; exit 2; }
sh0="$(specs_hash)"; m0="$(tip fix)"
run fix PII_INCIDENT_MERGE=0 PII_INCIDENT_WORKER="$WORKER"
m1="$(tip fix)"; brief="$scratch/fix/state/briefs/dirty-tree.md"
if [ -f "$brief" ] && grep -q 'dirty-tree' "$brief" && grep -q 'count: 4' "$brief" && grep -q 'first_seen: 2026-10-11T15:00:00Z' "$brief" \
   && grep -q 'cycle-x.log:12' "$brief" && [ "$(specs_hash)" = "$sh0" ]; then ok "an_incident_becomes_a_brief"; else bad "an_incident_becomes_a_brief"; fi
if git -C "$scratch/fix/repo" rev-parse -q --verify refs/heads/incident/dirty-tree >/dev/null && [ "$m0" = "$m1" ] && [ "$(cat "$scratch/fix/rc")" = 0 ]; then
  ok "a_fix_lands_on_a_branch_and_master_does_not_move (master ${m0:0:12} == ${m1:0:12})"; else bad "a_fix_lands_on_a_branch_and_master_does_not_move"; cat "$scratch/fix/out"; fi
legs="$(grep '^legs:' "$scratch/fix/out")"; echo "$legs"
if echo "$legs" | grep -q 'base_rc=[^0 ][0-9]* branch_rc=0 gate=PASS' || echo "$legs" | grep -Eq 'base_rc=[1-9][0-9]* branch_rc=0 gate=PASS'; then ok "the_proof_is_red_on_base_and_green_on_branch"; else bad "the_proof_is_red_on_base_and_green_on_branch"; fi

# 4. near-miss control: a test that already passes on base parks, keeps no branch, merges nothing
mk green 'grep -q fn crates/pii-server/src/daemon/mod.rs' || exit 2
g0="$(tip green)"; run green PII_INCIDENT_WORKER="$WORKER"
if [ "$(cat "$scratch/green/rc")" = 1 ] && grep -q 'OUTCOME=parked' "$scratch/green/out" && grep -q 'NOT red on the base' "$scratch/green/out" \
   && ! git -C "$scratch/green/repo" rev-parse -q --verify refs/heads/incident/dirty-tree >/dev/null && [ "$(tip green)" = "$g0" ]; then
  ok "a_test_already_green_on_base_parks"; else bad "a_test_already_green_on_base_parks"; cat "$scratch/green/out"; fi

# 5. a gate that is not green parks, naming the gate; no merge even with merge on
mk gate "$TEST" || exit 2
q0="$(tip gate)"; run gate PII_INCIDENT_WORKER="$WORKER" PII_INCIDENT_GATE=false
if [ "$(cat "$scratch/gate/rc")" = 1 ] && grep -q 'OUTCOME=parked.*gate' "$scratch/gate/out" && [ "$(tip gate)" = "$q0" ]; then ok "a_gate_that_is_not_green_parks"; else bad "a_gate_that_is_not_green_parks"; cat "$scratch/gate/out"; fi

# 6. the proven branch is the only thing that moves master: a fast-forward, nothing else
mk ff "$TEST" || exit 2
run ff PII_INCIDENT_WORKER="$WORKER"
b="$(git -C "$scratch/ff/repo" rev-parse incident/dirty-tree 2>/dev/null)"
if [ "$(cat "$scratch/ff/rc")" = 0 ] && [ -n "$b" ] && [ "$(tip ff)" = "$b" ] && git -C "$scratch/ff/repo" merge-base --is-ancestor "$base_sha" master; then ok "a_proven_branch_fast_forwards_master"; else bad "a_proven_branch_fast_forwards_master"; cat "$scratch/ff/out"; fi

if [ "$fails" -eq 0 ]; then echo "INCIDENT-BRANCH OK"; exit 0; fi
echo "INCIDENT-BRANCH FAILED ($fails)"; exit 1
