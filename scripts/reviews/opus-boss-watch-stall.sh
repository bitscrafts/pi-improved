#!/usr/bin/env bash
# The narrow review the operator asked for (2026-10-16): the boss watch's stall under load.
# RESUMES the previous opus session (f3a2a7df, the next-phases review) so the project state it already established is
# not re-derived and re-billed; the new brief is what changes.
set -uo pipefail
cd /projects/pi-improved
. scripts/autonomy/env.sh 2>/dev/null || true
O=/projects/_tmp/pii-review-20261016; B=$O/brief.md
SID_FILE=$O/session.txt
L=$O/opus-boss-watch-stall-$(date -u +%Y%m%dT%H%M%SZ).log
[ -f "$B" ] || { echo "REFUSED: brief missing: $B"; exit 2; }
SID="${1:-$(cat "$SID_FILE" 2>/dev/null)}"
if [ -n "$SID" ]; then RESUME=(--resume "$SID"); echo "== RESUMING $SID"; else RESUME=(); echo "== fresh session"; fi
echo "brief: $B"; echo "log:   $L"
claude -p "$(cat "$B")" "${RESUME[@]}" --permission-mode bypassPermissions --model claude-opus-5-5 \
  --output-format stream-json --verbose --include-partial-messages >"$L" 2>"$L.err"
RC=$?
grep -ao '"session_id":"[a-f0-9-]*"' "$L" | head -1 | cut -d'"' -f4 >"$SID_FILE"
echo "rc=$RC session=$(cat "$SID_FILE" 2>/dev/null) log=$L"
