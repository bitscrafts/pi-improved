#!/usr/bin/env bash
# probe.sh — is the Claude account answering RIGHT NOW?
#
# A 1-turn probe, cheap and decisive. Exit codes are the interface:
#   0  the window is open  (the account answered)
#   3  the window is CLOSED (a limit; the text that names it is printed, incl. any reset time)
#   4  the probe could not decide (a real error: auth, network, binary) — do NOT treat as closed
#
# Why a probe and not a blind dispatch: a spend limit has to be READ from the CLI's own message,
# never guessed or polled, and a long run must not be started on a closed window (the reset can be
# hours away). The probe costs one turn; the mistake costs a whole window of silence.
#
# Fixed 2026-10-11 by its own selftest: the first version passed the JSON BODY and the stderr TEXT
# to grep as if they were FILENAMES ("grep: {…}: File name too long" / "[ ! -s '' ]"), so a real
# error was misread as a closed window AND an OPEN window was misread as a probe error — a loop
# that would never have spent anything, discovered by the fake-CLI tests before any dispatch.
set -uo pipefail
OUT="$(mktemp)"; ERR="$(mktemp)"
trap 'rm -f "$OUT" "$ERR"' EXIT
timeout 180 claude -p "Reply with exactly: PONG55" --max-turns 1 --output-format json >"$OUT" 2>"$ERR"
RC=$?
BODY="$(cat "$OUT" 2>/dev/null || true)"
ERRTXT="$(cat "$ERR" 2>/dev/null || true)"

# 1) it answered — the model that answered is printed, never assumed
if [ "$RC" -eq 0 ] && printf '%s' "$BODY" | grep -q 'PONG55'; then
  MODEL="$(printf '%s' "$BODY" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(",".join(d.get("modelUsage",{}).keys()))' 2>/dev/null || true)"
  COST="$(printf '%s' "$BODY" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("total_cost_usd"))' 2>/dev/null || true)"
  printf 'OPEN model=%s cost=%s\n' "${MODEL:-?}" "${COST:-?}"
  exit 0
fi

# 2) a limit, in the CLI's own words ("resets 8:20pm (UTC)", "spend limit", 429 …)
ALL="$BODY
$ERRTXT"
if printf '%s' "$ALL" | grep -qiE 'session limit|spend limit|usage limit|rate.?limit|resets|quota|429'; then
  printf 'CLOSED — the CLI named a limit:\n'
  printf '%s' "$ALL" | grep -ioE '(You[^"]{0,30}(session|spend|usage) limit[^"]{0,120}|resets [^"]{0,60}|429[^"]{0,60})' | head -3
  printf 'rc=%s stderr_bytes=%s\n' "$RC" "$(printf '%s' "$ERRTXT" | wc -c)"
  exit 3
fi

# 3) the measured signature of a closed 5h window: rc!=0 with EMPTY stderr and no result body
if [ "$RC" -ne 0 ] && [ -z "$ERRTXT" ]; then
  printf 'CLOSED (rc=%s, empty stderr, no result) — the measured signature of a closed window\n' "$RC"
  exit 3
fi

# 4) anything else is an error, NOT a closed window: an error must never buy silence
printf 'UNDECIDED rc=%s: %s\n' "$RC" "$(printf '%s' "$ERRTXT" | head -c 300 | tr '\n' ' ')"
exit 4
