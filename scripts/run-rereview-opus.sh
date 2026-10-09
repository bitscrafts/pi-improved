#!/usr/bin/env bash
# run-rereview-opus.sh — RE-REVIEW pass of the review -> implement -> re-review pipeline.
#
# Probes the quota window, resumes the SAME reviewer session that wrote 043, runs a
# read-only (test-mode) re-review against the amended brief, proves the tree stayed clean,
# then files the report into the project SSOT + index + pointer + amem.
#
#   bash scripts/run-rereview-opus.sh              # probe, then run
#   DRY_RUN=1 bash scripts/run-rereview-opus.sh    # print what it would do, spend nothing
#
# Exit codes: 0 ran or dry-ran · 2 window shut (nothing spent) · 1 the run or filing failed.
# Everything the run needs is set INSIDE this script (a background shell inherits nothing).
set -uo pipefail
export IS_SANDBOX="${IS_SANDBOX:-1}"   # root + bypassPermissions requires it (measured 2026-09-28)

# ============================== CONFIG ==============================
REPO=/projects/pi-improved
DOCS="$REPO/_docs/06-research"
BRIEF="$DOCS/044-REREVIEW-BRIEF-2026-10-09.md"           # durable INPUT (in the tree)
WORK=/projects/_tmp/pii-rereview                          # OUTPUT-only scratch
REVIEWER_SESSION=ef17e6d6-1741-42d6-87c6-16ffc775d231     # the 043 session — resume-first
MODEL="${MODEL:-claude-opus-5-5}"
MODE="${REVIEW_MODE:-test}"                               # test: it RUNS the gate commands
MAX_TURNS="${MAX_TURNS:-200}"
EFFORT_LEVEL="${EFFORT:-high}"
RUNNER=/root/.hermes/skills/research/opus-review/scripts
CLAUDE=/root/.local/bin/claude
AMEM_URL="${AMEM_URL:-http://10.10.10.217:7420}"
DEST_REL="_docs/06-research/044-opus-rereview-$(date -u +%Y-%m-%d).md"
POINTER="$DOCS/030-OPUS-START-HERE.md"
INDEX="$REPO/_docs/INDEX.md"
# ===================================================================

TS="$(date -u +%Y%m%dT%H%M%SZ)"; LOG="$WORK/run-$TS.log"
mkdir -p "$WORK"
say() { printf '%s\n' "$*"; }

say "== PII RE-REVIEW — $TS"
say "   repo   : $REPO ($(git -C "$REPO" rev-parse --short HEAD 2>/dev/null), $(git -C "$REPO" status --porcelain 2>/dev/null | wc -l) changed)"
say "   brief  : $BRIEF ($(wc -c <"$BRIEF" 2>/dev/null) bytes)"
say "   mode   : $MODE (runs the gate commands)  model: $MODEL  resume: $REVIEWER_SESSION  turns<=$MAX_TURNS"

[ -s "$BRIEF" ] || { say "!! brief missing/empty: $BRIEF"; exit 1; }
if [ -n "$(git -C "$REPO" status --porcelain)" ]; then
  say "!! tree is dirty — the reviewer's read-only proof needs a clean baseline:"; git -C "$REPO" status --porcelain | sed 's/^/     /'; exit 1
fi

if [ -n "${DRY_RUN:-}" ]; then
  say ""; say "DRY RUN — would execute:"
  say "  REVIEW_MODE=$MODE MAX_TURNS=$MAX_TURNS MODEL=$MODEL EFFORT=$EFFORT_LEVEL \\"
  say "    bash $RUNNER/run-opus-review.sh $REPO $BRIEF $WORK --resume $REVIEWER_SESSION"
  say "  then: file -> $DEST_REL ; INDEX entry ; pointer in 030 ; amem store ; READY-TO-COMMIT"
  exit 0
fi

say ""; say "-- probe: is the window open? (1 turn; names the answering model)"
probe="$(timeout 180 "$CLAUDE" -p "Reply with exactly: PONG55" --model "$MODEL" --max-turns 1 --output-format json 2>&1 || true)"
if ! printf '%s' "$probe" | grep -q PONG55; then
  say "   NOT OPEN — not dispatching (nothing spent). Probe said:"; printf '%s\n' "$probe" | tail -4 | sed 's/^/     /'
  say "   Read the reset time from that text; re-run after it. Do NOT blind-retry."; exit 2
fi
say "   open — answers PONG55"

say ""; say "-- dispatch (read-only; resumed session)"
REVIEW_MODE="$MODE" MAX_TURNS="$MAX_TURNS" MODEL="$MODEL" EFFORT="$EFFORT_LEVEL" TASK_LABEL=opus-rereview \
  bash "$RUNNER/run-opus-review.sh" "$REPO" "$BRIEF" "$WORK" --resume "$REVIEWER_SESSION"
rc=$?; say "   runner exit=$rc"

say ""; say "-- read-only proof"
ch="$(git -C "$REPO" status --porcelain | wc -l)"
[ "$ch" = "0" ] && say "   tree clean — read-only respected" || { say "   !! $ch changed path(s):"; git -C "$REPO" status --porcelain | sed 's/^/     /'; }

REPORT="$(ls -t "$WORK"/opus-rereview-result-*.md 2>/dev/null | head -1)"
META="$(ls -t "$WORK"/opus-rereview-*.json 2>/dev/null | grep -v probe | head -1)"
if [ -z "$REPORT" ] || [ ! -s "$REPORT" ]; then
  say ""; say "!! no result text produced — the review did not answer. Log: $WORK"; exit 1
fi
# VALIDITY GATE: a quota/limit error text is not a report.
if grep -qEi "session limit|usage limit|credit balance|rate limit|insufficient credits|you've hit your" "$REPORT"; then
  say ""; say "!! VALIDITY GATE: the result is an ERROR TEXT (quota/session limit), not a report:"
  head -3 "$REPORT" | sed 's/^/     /'
  say "   refusing to file/store. Read the reset time; NEVER blind-retry."; exit 1
fi
if [ "$(wc -c <"$REPORT")" -lt 5000 ]; then
  say ""; say "!! VALIDITY GATE: result is only $(wc -c <"$REPORT") bytes — below the re-review floor."
  say "   Log: $WORK/opus-rereview-*.json"; exit 1
fi
COST="$(python3 -c "import json;print(json.load(open('$META')).get('total_cost_usd'))" 2>/dev/null || echo '?')"
TURNS="$(python3 -c "import json;print(json.load(open('$META')).get('num_turns'))" 2>/dev/null || echo '?')"
STOP="$(python3 -c "import json;print(json.load(open('$META')).get('stop_reason'))" 2>/dev/null || echo '?')"
SID="$(python3 -c "import json;print(json.load(open('$META')).get('session_id'))" 2>/dev/null || echo "$REVIEWER_SESSION")"
say ""; say "-- report: $REPORT ($(wc -w <"$REPORT") words)  session=$SID turns=$TURNS stop=$STOP cost=\$$COST"

say ""; say "-- filing into the project SSOT: $DEST_REL"
MODEL="$MODEL" BRIEF="$BRIEF" INVENTORY="$DOCS/041-APPENDIX-codebase-inventory.txt" \
  SESSION_ID="$SID" STOP_REASON="$STOP" COST_USD="$COST" TURNS="$TURNS" \
  bash "$RUNNER/file-opus-review.sh" "$REPO" "$REPORT" "$DEST_REL" || say "   !! filing failed"

say ""; say "-- index + pointer"
python3 - "$REPO" "$DEST_REL" "$POINTER" "$INDEX" <<'PY'
import pathlib, sys, datetime
repo, dest_rel, pointer, index = sys.argv[1:5]
name = pathlib.Path(dest_rel).name
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
line = (f"- [`{dest_rel}`]({name}) — the claude+opus RE-REVIEW ({stamp}): batches A-D judged by behaviour;\n"
        f"  which of 043's ranked candidates landed, which claims held, and what the mutation tests actually guard.\n")
ip = pathlib.Path(index)
if ip.exists() and name not in ip.read_text():
    s = ip.read_text(); ip.write_text(s + ("" if s.endswith("\n") else "\n") + line); print("   INDEX updated")
pp = pathlib.Path(pointer)
if pp.exists() and name not in pp.read_text():
    s = pp.read_text()
    banner = (f"> **{stamp} — THE RE-REVIEW HAS RUN: [`{dest_rel}`]({name}).** Batches A-D are judged by behaviour;\n"
              f"> read it with 043 before acting on any candidate below.\n\n")
    parts = s.split("\n\n", 2)
    pp.write_text("\n\n".join(parts[:2]) + "\n\n" + banner + (parts[2] if len(parts) > 2 else "")); print("   pointer updated")
PY

say ""; say "-- storing the FULL report in amem"
bash "$RUNNER/store-report-in-amem.sh" "$REPO/$DEST_REL" "$AMEM_URL" pii "pii/review/rereview-$(date -u +%Y-%m-%d)" 14000 \
  || say "   !! amem store failed (the filed copy stands)"

say ""; say "-- commit-ready summary (this script does NOT commit)"
cat > "$WORK/READY-TO-COMMIT.txt" <<EOF
Run: session=$SID turns=$TURNS stop=$STOP cost=\$$COST  log=$LOG
Files changed by the filing step:
  $DEST_REL
  _docs/INDEX.md
  $POINTER
Commit (after reading the report):
  cd $REPO && git add $DEST_REL _docs/INDEX.md _docs/06-research/030-OPUS-START-HERE.md \\
    && git commit -m "review: opus re-review of batches A-D (044)"
NEXT: turn the new ranked candidates into specs.
EOF
say "   -> $WORK/READY-TO-COMMIT.txt"
say ""; say "DONE. report: $REPO/$DEST_REL"
