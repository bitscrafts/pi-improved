#!/usr/bin/env bash
# opus-next-phases.sh — the operator's ask (2026-10-15): given the original plan and the previous opus reviews,
# report the NEXT PHASES on the path to production. Fresh session on purpose: the input is a curated document set,
# and re-sending a 600 KB old session costs more than reading these documents does.
set -uo pipefail
cd /projects/pi-improved
. scripts/autonomy/env.sh 2>/dev/null || true     # IS_SANDBOX=1, or claude refuses bypassPermissions as root
OUT=/projects/_tmp/pii-review-20261015
BRIEF=$OUT/brief-next-phases.md
L=$OUT/opus-next-phases-$(date -u +%Y%m%dT%H%M%SZ).log
[ -f "$BRIEF" ] || { echo "REFUSED: brief missing: $BRIEF"; exit 2; }
echo "brief: $BRIEF"; echo "log:   $L"
claude -p "$(cat "$BRIEF")" --permission-mode bypassPermissions --model claude-opus-5-5 \
  --output-format stream-json --verbose --include-partial-messages >"$L" 2>"$L.err"
echo "rc=$? log=$L"
