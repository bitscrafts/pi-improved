#!/usr/bin/env bash
# Harness for spec 151 R4: one dispatch per fingerprint per local day, and a recorded cost ceiling.
# Drives scripts/autonomy/incident-fix.sh on a scratch repo with a worker that fixes nothing (the dispatch is what
# is counted). Usage: incident-budget.sh   (last line: BUDGET OK; exit 0 only then)
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
tmp=/projects/_tmp; mkdir -p "$tmp" || exit 2
s="$(mktemp -d "$tmp/incident-budget.XXXXXX")" || exit 2
fails=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fails=$((fails + 1)); }
git init -q "$s/repo" && git -C "$s/repo" -c user.name=t -c user.email=t@t commit -q --allow-empty -m base || exit 2
git -C "$s/repo" branch -M master
mkdir -p "$s/state"
for fp in alpha beta; do
  printf '{"fingerprint":"%s","first_seen":"2026-10-11T15:00:00Z","count":3,"evidence":"x.log:1","source":"cycle","test":"false"}\n' "$fp"
done >"$s/state/incidents.jsonl"
fix() { # fix <fp> <day> [extra env...]
  local fp="$1" day="$2"; shift 2
  env PII_REPO="$s/repo" PII_INCIDENT_STATE="$s/state" PII_INCIDENTS="$s/state/incidents.jsonl" PII_INCIDENT_DAY="$day" \
    PII_INCIDENT_WORKER='echo 0.7 >"$PII_INCIDENT_COST_FILE"' "$@" bash "$root/scripts/autonomy/incident-fix.sh" "$fp" 2>&1
}
o1="$(fix alpha 2026-10-14)"; echo "$o1" | grep -q 'OUTCOME=refused' && bad "the first dispatch must run" || ok "the first dispatch ran"
o2="$(fix alpha 2026-10-14)"; echo "$o2" | grep -q 'OUTCOME=refused.*one per fingerprint per local day' && ok "a_second_dispatch_in_the_day_is_refused" || { bad "a_second_dispatch_in_the_day_is_refused"; echo "$o2"; }
grep -q 'refused already dispatched on 2026-10-14' "$s/state/incidents/alpha/refusals.log" 2>/dev/null && ok "the refusal and its reason are recorded" || bad "the refusal is not recorded"
o3="$(fix beta 2026-10-14)"; echo "$o3" | grep -q 'OUTCOME=refused' && bad "near-miss: a different fingerprint the same day was refused" || ok "near-miss: a different fingerprint the same day is still dispatched"
o4="$(fix alpha 2026-10-15)"; echo "$o4" | grep -q 'OUTCOME=refused' && bad "a new local day was refused" || ok "a new local day proceeds"
# cost ceiling: alpha has now recorded 0.7 + 0.7; a ceiling of 1 refuses even on a fresh day, with the reason
o5="$(fix alpha 2026-10-16 PII_INCIDENT_COST_CEILING=1)"; echo "$o5" | grep -q 'OUTCOME=refused.*cost ceiling' && ok "the cost ceiling refuses, with its reason" || { bad "the cost ceiling did not refuse"; echo "$o5"; }
if [ "$fails" -eq 0 ]; then echo "BUDGET OK"; exit 0; fi
echo "BUDGET FAILED ($fails)"; exit 1
