#!/usr/bin/env bash
# control.sh — the loop's switchboard. Usable from Discord (the assistant runs it) or a terminal.
#
# The user's requirement: "make it controlable via discord". A Discord message reaches the assistant, which
# runs one of these verbs in /projects/pi-improved; nothing else is needed for the loop to obey, because both
# tick.sh (which starts cycles) and cycle.sh (which spends) read the control file before acting.
#
#   status                       what the loop is doing, what is queued, what the last cycle ended with
#   pause [reason]               no NEW cycle starts; a running cycle finishes what it started
#   resume                       leave PAUSED
#   skip <item>                  mark one item skipped (it will not be picked again)
#   park <item>                  mark one item parked (needs a human/supervisor decision)
#   unskip <item>                put an item back to pending
#   ask <text>                   raise a question for the user; the next tick carries it to their phone
#   answer <n> <text>            record the user's answer to question n (the tick stops repeating it)
#   note <text>                  append a dated note for the supervisor/operator to read
#   log [n]                      the last n lines of the newest cycle log (default 20)
set -uo pipefail
cd /projects/pi-improved || { echo "cannot cd to /projects/pi-improved"; exit 1; }
ST=_tmp/autonomy; mkdir -p "$ST"; CTL=$ST/control
TS=$(date -u +%FT%TZ)

case "${1:-status}" in
  pause)
    printf 'PAUSED by %s at %s: %s\n' "${PII_CONTROL_BY:-human}" "$TS" "${2:-no reason given}" >"$CTL"
    echo "pii autonomy: PAUSED — ${2:-no reason given}"
    echo "(a cycle already running finishes; no new one starts)"
    ;;
  resume)
    printf 'RUNNING at %s (resumed by %s)\n' "$TS" "${PII_CONTROL_BY:-human}" >"$CTL"
    echo "pii autonomy: RUNNING — the next tick starts a cycle"
    ;;
  skip|park|unskip)
    ID=${2:?$1 needs an item id (e.g. A2)}
    case "$1" in
      skip)   python3 scripts/autonomy/state.py set "$ID" status=skipped "last=skipped $TS" ;;
      park)   python3 scripts/autonomy/state.py set "$ID" status=parked  "last=parked $TS" ;;
      unskip) python3 scripts/autonomy/state.py set "$ID" status=pending "last=re-queued $TS" ;;
    esac
    echo "pii autonomy: item $ID -> $1"; python3 scripts/autonomy/state.py get "$ID" | head -4
    ;;
  ask)
    printf -- '- [ ] %s | %s\n' "$TS" "${2:?ask needs text}" >>"$ST/questions.md"
    echo "pii autonomy: question raised — the next tick carries it to the phone"
    tail -1 "$ST/questions.md"
    ;;
  answer)
    ID=${2:?answer needs the question number (see 'status')}; TXT=${3:?answer needs the text}
    python3 - "$ID" "$TXT" <<'PY'
import sys, pathlib
n, txt = sys.argv[1], sys.argv[2]
p = pathlib.Path("_tmp/autonomy/questions.md")
lines = p.read_text().splitlines()
open_q = [i for i, l in enumerate(lines) if l.startswith("- [ ]")]
if not open_q:
    print("no open question"); raise SystemExit(1)
try:
    i = open_q[int(n) - 1]
except (ValueError, IndexError):
    print(f"question {n} does not exist (there are {len(open_q)} open)"); raise SystemExit(1)
lines[i] = lines[i].replace("- [ ]", "- [x]", 1) + f"  ==> ANSWERED: {txt}"
p.write_text("\n".join(lines) + "\n")
print("answered:", lines[i][:120])
PY
    ;;
  note)
    printf '%s | %s\n' "$TS" "${2:?note needs text}" >>"$ST/notes.md"
    echo "pii autonomy: noted"
    ;;
  log)
    L=$(ls -t "$ST"/logs/cycle-*.log 2>/dev/null | head -1)
    [ -n "$L" ] && tail -"${2:-20}" "$L" || echo "(no cycle log yet)"
    ;;
  status|*)
    echo "== control: $(cat "$CTL" 2>/dev/null || echo 'RUNNING (default)')"
    if flock -n "$ST/lock" true 2>/dev/null; then echo "== cycle: none running"; else
      echo "== cycle: IN FLIGHT for $(( ( $(date +%s) - $(stat -c %Y "$ST/lock") ) / 60 )) min"; fi
    echo "== queue:"; python3 scripts/autonomy/state.py list | awk -F'\t' '{printf "   %-4s %-52s %s\n", $1, substr($3,1,52), $4}'
    L=$(ls -t "$ST"/logs/cycle-*.log 2>/dev/null | head -1)
    [ -n "$L" ] && { echo "== last cycle: ${L##*/}"; grep -o 'OUTCOME=[^ ]*\|ITEM=[^ ]*' "$L" | tail -2 | tr '\n' ' '; echo; }
    Q="$ST/questions.md"
    if [ -f "$Q" ] && grep -q '^- \[ \]' "$Q"; then
      echo "== open questions (the tick is carrying these to the phone):"
      grep -n '^- \[ \]' "$Q" | sed 's/^/   /' | cut -c1-140
    fi
    echo "== HEAD: $(git log --oneline -1 | cut -c1-64)"
    echo "== jobs: pii autonomy loop (*/30, no model) and pii autonomy supervisor (every 4h, claude)"
    ;;
esac
