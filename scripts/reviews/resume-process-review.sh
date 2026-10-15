#!/usr/bin/env bash
# resume-process-review.sh - the operator's ask (2026-10-15): review the PROCESS itself.
# RESUME-FIRST, mechanically: this resumes the EXISTING review session; it refuses to start a fresh one.
set -uo pipefail
cd /projects/pi-improved
. scripts/autonomy/env.sh   # IS_SANDBOX=1 and the provider env: without it claude refuses bypassPermissions as root
SID=ef17e6d6-1741-42d6-87c6-16ffc775d231
BR=/projects/_tmp/pii-review-20261015/brief-process-review.md
OUT=/projects/_tmp/pii-review-20261015
mkdir -p "$OUT"
[ -n "$SID" ] || { echo "REFUSED: no session id to resume - a fresh review is not what this does"; exit 0; }
[ -f "$BR" ] || { echo "REFUSED: brief missing: $BR"; exit 0; }
L="$OUT/opus-process-review-$(date -u +%Y%m%dT%H%M%SZ).log"
echo "resuming $SID with $BR -> $L"
claude --resume "$SID" -p "$(cat "$BR")" --permission-mode bypassPermissions --model claude-opus-5-5 --output-format stream-json --verbose --include-partial-messages >"$L" 2>"$L.err"
echo "rc=$? log=$L"
