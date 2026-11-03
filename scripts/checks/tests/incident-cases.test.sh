#!/usr/bin/env bash
# incident-cases.sh, tested without a host: a new incident opens a case, a case is not a row, the cap holds, and the
# cron is silent when nothing is new. Every run writes under _tmp/ (never specs/), and the trap removes only that scratch.
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd) || exit 1
cd "$ROOT" || exit 1
S=scripts/autonomy/incident-cases.sh
ok=0; bad=0
pass(){ ok=$((ok+1)); echo "ok   $1"; }
fail(){ bad=$((bad+1)); echo "FAIL $1"; }
mkdir -p "$ROOT/_tmp"
W=$(mktemp -d "$ROOT/_tmp/incident-cases-XXXXXX")
trap 'case "$W" in "$ROOT"/_tmp/*) find "$W" -mindepth 1 -delete; rmdir "$W" ;; esac' EXIT
export PII_CASE_SPECS="$W/specs" PII_PLAN="$W/plan.tsv"
mkdir -p "$PII_CASE_SPECS"
printf 'A1\t999-demo\tbrief\ttitle\ttrue\tpath\t1\n' >"$PII_PLAN"

# a scan line as `pii incidents scan --source runtime` prints it
inc(){ printf 'incident %s count=%s first_seen=%s last_seen=%s latency_secs=60 window_secs=3600 source=runtime\n' "$1" "$2" "$3" "$3"; }
fresh(){ export PII_CASES="$W/$1.tsv" PII_CASE_EVIDENCE="$W/$1-evidence"; }
# has <fingerprint> <first_seen> <count> <state> <case_id>: the ledger holds exactly this line
has(){ awk -F'\t' -v f="$1" -v s="$2" -v c="$3" -v st="$4" -v id="$5" \
  'NF==6 && $1==f && $2==s && $3==c && $5==st && $6==id {ok=1} END{exit !ok}' "$PII_CASES" 2>/dev/null; }

inc 'refusal-repeated:channel.poll.failed' 4 2026-11-01T08:00:00Z >"$W/two.txt"
inc 'observability-lost:r000000004577' 1 2026-11-02T06:10:00Z >>"$W/two.txt"

# 1. a new incident becomes a new case
fresh t1
out=$(bash "$S" --input "$W/two.txt" 2>&1); rc=$?
[ "$rc" -eq 0 ] && has 'refusal-repeated:channel.poll.failed' 2026-11-01T08:00:00Z 4 new 'refusal-repeated:channel.poll.failed' \
  && has 'observability-lost:r000000004577' 2026-11-02T06:10:00Z 1 new 'observability-lost:r000000004577' \
  && [ "$(printf '%s\n' "$out" | grep -c '^new ')" -eq 2 ] \
  && grep -q 'first_seen=2026-11-02T06:10:00Z' "$PII_CASE_EVIDENCE/observability-lost:r000000004577.txt" \
  && pass "a_new_incident_becomes_a_new_case" || fail "a_new_incident_becomes_a_new_case (rc=$rc out=$out)"

# 2. CONTROL: the same run again appends nothing and prints nothing
before=$(cat "$PII_CASES")
out=$(bash "$S" --input "$W/two.txt" 2>&1); rc=$?
[ "$rc" -eq 0 ] && [ -z "$out" ] && [ "$before" = "$(cat "$PII_CASES")" ] \
  && pass "CONTROL_the_same_run_appends_nothing" || fail "CONTROL_the_same_run_appends_nothing (rc=$rc out=$out)"

# 3. a closed fingerprint that recurs is its own case, <fp>#2
fresh t3
printf 'refusal-repeated:channel.poll.failed\t2026-10-20T00:00:00Z\t2\tmvcorrea@lnx\tclosed\trefusal-repeated:channel.poll.failed\n' >"$PII_CASES"
out=$(bash "$S" --input "$W/two.txt" 2>&1)
has 'refusal-repeated:channel.poll.failed' 2026-11-01T08:00:00Z 4 new 'refusal-repeated:channel.poll.failed#2' \
  && [ "$(printf '%s\n' "$out" | grep -c 'refusal-repeated:channel.poll.failed#2')" -eq 1 ] \
  && pass "a_recurrence_after_close_is_its_own_case" || fail "a_recurrence_after_close_is_its_own_case (out=$out)"

# 4. a case is not dispatchable: cycle.sh's pick (state.py pick) reads plan.tsv and ignores every case
fresh t4
before=$(python3 scripts/autonomy/state.py pick 2>&1); planbefore=$(cat "$PII_PLAN")
bash "$S" --input "$W/two.txt" >/dev/null 2>&1
after=$(python3 scripts/autonomy/state.py pick 2>&1)
[ "$before" = "$after" ] && [ "$planbefore" = "$(cat "$PII_PLAN")" ] && ! grep -q 'observability-lost\|refusal-repeated' <<<"$after" \
  && pass "a_case_is_not_dispatchable" || fail "a_case_is_not_dispatchable (before=$before after=$after)"

# 5. CONTROL: --brief writes the spec stub with the evidence and flips the state to briefed
fresh t5
bash "$S" --input "$W/two.txt" >/dev/null 2>&1
listed=$(bash "$S" --list 2>&1)
bash "$S" --brief 'observability-lost:r000000004577' >/dev/null 2>&1; rc=$?
spec="$PII_CASE_SPECS/auto-observability-lost:r000000004577.md"
[ "$rc" -eq 0 ] && [ -f "$spec" ] && grep -q 'observability-lost:r000000004577' "$spec" \
  && grep -q 'first_seen=2026-11-02T06:10:00Z' "$spec" && grep -q 'count=1' "$spec" && grep -q '^## TODO criteria' "$spec" \
  && has 'observability-lost:r000000004577' 2026-11-02T06:10:00Z 1 briefed 'observability-lost:r000000004577' \
  && grep -q 'observability-lost:r000000004577' <<<"$listed" && ! grep -q 'observability-lost' <<<"$(bash "$S" --list 2>&1)" \
  && pass "CONTROL_briefing_a_case_writes_a_spec_and_flips_the_state" \
  || fail "CONTROL_briefing_a_case_writes_a_spec_and_flips_the_state (rc=$rc)"

# 6. the cap: nine new incidents, --max-new 2 -> the two oldest appended, seven reported as deferred
fresh t6
: >"$W/nine.txt"
for d in 1 2 3 4 5 6 7 8 9; do inc "flood-$d" 1 "2026-10-0${d}T00:00:00Z" >>"$W/nine.txt"; done
out=$(bash "$S" --input "$W/nine.txt" --max-new 2 2>&1); rc=$?
[ "$rc" -eq 0 ] && [ "$(awk 'NF==6' "$PII_CASES" | wc -l)" -eq 2 ] \
  && has flood-1 2026-10-01T00:00:00Z 1 new flood-1 && has flood-2 2026-10-02T00:00:00Z 1 new flood-2 \
  && [ "$(grep -c '^deferred ' <<<"$out")" -eq 7 ] \
  && pass "the_cap_keeps_a_backlog_from_flooding" || fail "the_cap_keeps_a_backlog_from_flooding (rc=$rc out=$out)"

# 7. dry-run: rc 0, prints what it would open, and the existing ledger stays byte-identical
fresh t7
printf 'refusal-repeated:channel.poll.failed\t2026-11-01T08:00:00Z\t4\tmvcorrea@lnx\tnew\trefusal-repeated:channel.poll.failed\n' >"$PII_CASES"
cp "$PII_CASES" "$W/t7.before"
out=$(bash "$S" --input "$W/two.txt" --dry-run 2>&1); rc=$?
[ "$rc" -eq 0 ] && cmp -s "$W/t7.before" "$PII_CASES" && grep -q 'observability-lost:r000000004577' <<<"$out" \
  && ! grep -q 'refusal-repeated' <<<"$out" && [ ! -e "$PII_CASE_EVIDENCE/observability-lost:r000000004577.txt" ] \
  && pass "dry_run_writes_nothing" || fail "dry_run_writes_nothing (rc=$rc out=$out)"

echo "INCIDENT-CASES: $ok passed, $bad failed"
[ "$bad" -eq 0 ]
