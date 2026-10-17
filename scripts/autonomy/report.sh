#!/usr/bin/env bash
# report.sh — the loop's lifecycle lines as pii events (spec 173). Sourced by cycle.sh; needs CID.
#   report <kind> [detail]   one `pii loop emit`. FAIL-OPEN: it never stops the loop and always returns 0.
#                            When the daemon did not answer (pii exits non-zero) the cycle is written to
#                            $PII_LOOP_STATE/unreported, and the NEXT cycle's first report says so.
# Read it back with:  pii loop events --last 20 [--log <daemon log>]
PII_LOOP_STATE=${PII_LOOP_STATE:-/projects/pi-improved/_tmp/autonomy}
export PII_LOOP_EVENTS=${PII_LOOP_EVENTS:-$PII_LOOP_STATE/loop-events.ndjson}
if [ -z "${PII_BIN:-}" ]; then
  PII_BIN=/projects/pi-improved/target/debug/pii; [ -x "$PII_BIN" ] || PII_BIN=$(command -v pii || echo pii)
fi
report(){
  local kind=$1 detail=${2:-} mark=$PII_LOOP_STATE/unreported
  if [ -s "$mark" ]; then
    printf '[%s] pii: cycle(s) not reported to the daemon: %s\n' "$(date -u +%H:%M:%SZ)" "$(tr '\n' ' ' <"$mark")"
    detail="$detail previous-not-reported=$(tr '\n' ',' <"$mark")"
    mv "$mark" "$mark.seen-$(date -u +%Y%m%dT%H%M%SZ)" 2>/dev/null || true
  fi
  mkdir -p "$PII_LOOP_STATE" 2>/dev/null || true
  if ! timeout 15 "$PII_BIN" loop emit --cycle "${CID:-unknown}" --kind "$kind" --detail "$detail" >/dev/null 2>&1; then
    grep -qx "${CID:-unknown}" "$mark" 2>/dev/null || echo "${CID:-unknown}" >>"$mark" 2>/dev/null || true
  fi
  return 0
}
