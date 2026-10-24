#!/usr/bin/env bash
# classify-failure.test.sh — the cases for classify-failure.sh (spec 254). Exits 0 only if every case holds.
cd "$(dirname "$0")/../../.." || exit 1
C=scripts/autonomy/classify-failure.sh
FAILS=0
# check <name> <stdin text> <expected word>: the text is passed literally, never through printf's escape handling
check(){
  local got
  got=$(printf '%s' "$2" | bash "$C")
  if [ "$got" = "$3" ]; then echo "ok $1"; else echo "FAIL $1"; echo "  got '$got', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
check "auth_revoked" 'Failed to authenticate: OAuth token revoked. Please log in again' auth
check "auth_status_401" '{"is_error":true,"api_error_status":401}' auth
check "quota_spend" "You've hit your monthly spend limit. Resets 4:00pm" quota
check "auth_beats_quota" "Please log in again. You've hit your monthly spend limit." auth
check "plain_work_failure" 'error[E0063]: missing field' work
check "empty_is_work" '' work
check "auth_journal_status_only" '{"claude_api_error_status":"401","claude_result":"","rc":"1"}' auth
check "NEAR-MISS: journal status that is not 401" '{"claude_api_error_status":"500","claude_result":"","rc":"1"}' work
check "NEAR-MISS: 401 as a plain number" 'took 401 ms' work
check "NEAR-MISS: the word limit alone" 'line limit of 450 reached' work
[ "$FAILS" -eq 0 ]
