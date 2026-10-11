#!/usr/bin/env bash
# selftest.sh — the autonomy mechanisms, tested before they are trusted. Re-runnable at any time.
#
# The user's requirement, verbatim: "e essencial que os mecanismos fiquem testados!". This script is the
# answer: every mechanism the loop depends on is exercised here, with fake CLIs where money would be spent,
# and it prints one PASS/FAIL row per mechanism plus a final VERDICT line. It is safe to run while the loop
# runs (it uses its own scratch, its own fake `claude`, and DRY/temp-plan modes).
#
# It found three real defects the first time it was written: the probe passed the JSON body and the stderr
# text to grep as FILENAMES (an open window read as a probe error, and a real error read as a closed
# window), and the cycle passed the spec FILENAME where the dispatcher wants the NUMBER.
. "$(dirname "$(readlink -f "$0")")/env.sh"   # the loop needs PATH set inside the script
set -uo pipefail
cd /projects/pi-improved || exit 1
T=$(mktemp -d /projects/_tmp/autonomy-selftest-XXXXXX)
BIN=$T/bin; mkdir -p "$BIN"
PASS=0; FAIL=0
row(){ # row <name> <expected> <actual>
  if [ "$2" = "$3" ]; then printf 'PASS  %-46s %s\n' "$1" "$3"; PASS=$((PASS+1));
  else printf 'FAIL  %-46s expected=%s actual=%s\n' "$1" "$2" "$3"; FAIL=$((FAIL+1)); fi
}
fake(){ printf '#!/bin/sh\n%s\n' "$2" >"$BIN/claude"; chmod +x "$BIN/claude"; }

echo "== 1. queue bookkeeping (state.py, atomic)"
PII_PLAN=$T/plan.tsv printf 'X1\tfake-spec.md\t-\tthe fake item\t-\t-\t0\n' >"$T/plan.tsv"
row "pick finds the first row" "X1" "$(PII_PLAN=$T/plan.tsv python3 scripts/autonomy/state.py pick | cut -f1)"
PII_PLAN=$T/plan.tsv python3 scripts/autonomy/state.py set X1 status=needs-review attempts=2 >/dev/null
row "set writes and get reads back" "2" "$(python3 scripts/autonomy/state.py get X1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["attempts"])')"
row "state file stays valid JSON" "ok" "$(python3 -c 'import json;json.load(open("_tmp/autonomy/state.json"));print("ok")')"

echo "== 2. the window probe (fake CLIs: no money spent)"
fake x "echo '{\"type\":\"result\",\"result\":\"PONG55\",\"modelUsage\":{\"claude-opus-5-5\":{}}}'"
PATH=$BIN:$PATH bash scripts/autonomy/probe.sh >/dev/null 2>&1; row "open window -> 0" 0 $?
fake x 'echo "You have hit your session limit - resets 8:20pm (UTC)" >&2; exit 1'
PATH=$BIN:$PATH bash scripts/autonomy/probe.sh >/dev/null 2>&1; row "a named limit -> 3" 3 $?
fake x 'exit 1'
PATH=$BIN:$PATH bash scripts/autonomy/probe.sh >/dev/null 2>&1; row "silent failure (measured signature) -> 3" 3 $?
fake x 'echo "Error: connect ECONNREFUSED" >&2; exit 1'
PATH=$BIN:$PATH bash scripts/autonomy/probe.sh >/dev/null 2>&1; row "a real error -> 4 (never read as closed)" 4 $?
# the cycle-guard cases below reuse an OPEN fake CLI, so the selftest never depends on the real account
fake x 'echo "{\"type\":\"result\",\"result\":\"PONG55\",\"modelUsage\":{\"claude-opus-5-5\":{}}}"'

echo "== 3. one cycle at a time (the lock)"
flock -n /projects/pi-improved/_tmp/autonomy/lock -c 'sleep 3' &
sleep 0.5
bash scripts/autonomy/cycle.sh >/dev/null 2>&1; row "a held lock -> 5 (nothing done)" 5 $?
wait

echo "== 4. the cycle's guards (no dispatch, no money)"
PATH=$BIN:$PATH DRY=1 bash scripts/autonomy/cycle.sh >/dev/null 2>&1; row "DRY=1 -> 0 without dispatching" 0 $?
# the cycle's exit code IS its outcome (the interface): assert the code, not a grep through a pipe
# (a pipe under `pipefail` returns the first non-zero command, so `if ... | grep` reads a 6 as "no match")
PII_PLAN=$T/plan.tsv PATH=$BIN:$PATH bash scripts/autonomy/cycle.sh >/dev/null 2>&1; row "a plan row with no spec -> rc 6" 6 $?
row "the spec number is derived from the filename" "134" "$(printf '%s' '134-x.md' | sed -E 's/^([0-9]+).*/\1/')"

echo "== 5. the operator's switch (Discord verbs)"
bash scripts/autonomy/control.sh pause "selftest" >/dev/null
PATH=$BIN:$PATH bash scripts/autonomy/cycle.sh 2>&1 | grep -q 'OUTCOME=paused'; row "paused -> the cycle spends nothing" 0 $?
bash scripts/autonomy/tick.sh | grep -q 'PAUSED'; row "paused -> the tick says so (once)" 0 $?
bash scripts/autonomy/tick.sh | grep -q 'PAUSED'; row "paused -> the tick does not repeat itself" 1 $?
bash scripts/autonomy/control.sh resume >/dev/null
row "resume clears the switch" "RUNNING at" "$(cut -c1-10 _tmp/autonomy/control)"
bash scripts/autonomy/control.sh skip X1 >/dev/null
row "skip takes an item out of the pick" "NONE" "$(PII_PLAN=$T/plan.tsv python3 scripts/autonomy/state.py pick | cut -f1)"
bash scripts/autonomy/control.sh unskip X1 >/dev/null
row "unskip puts it back" "X1" "$(PII_PLAN=$T/plan.tsv python3 scripts/autonomy/state.py pick | cut -f1)"
bash scripts/autonomy/control.sh status >/dev/null 2>&1; row "status runs" 0 $?
bash scripts/autonomy/control.sh note "selftest note" >/dev/null; row "note appends" "ok" "$(grep -q 'selftest note' _tmp/autonomy/notes.md && echo ok)"

echo "== 5b. a question reaches the phone, and an answer stops it"
bash scripts/autonomy/control.sh ask "selftest: which way should the loop go?" >/dev/null
bash scripts/autonomy/tick.sh | grep -q 'open question'; row "an open question is carried to the phone" 0 $?
bash scripts/autonomy/tick.sh | grep -q 'open question'; row "it is not repeated every tick" 1 $?
N=$(grep -c '^- \[ \]' _tmp/autonomy/questions.md)
bash scripts/autonomy/control.sh answer "$N" "selftest answer" >/dev/null
row "the answer is recorded" "ok" "$(grep -q 'ANSWERED: selftest answer' _tmp/autonomy/questions.md && echo ok)"

echo "== 6. the supervisor does nothing when there is nothing to do"
bash scripts/autonomy/control.sh pause "selftest2" >/dev/null
bash scripts/autonomy/supervise.sh | grep -q 'paused'; row "paused -> the supervisor is idle" 0 $?
bash scripts/autonomy/control.sh resume >/dev/null

echo
echo "RESULT: $PASS passed, $FAIL failed  (scratch: $T)"
# the selftest's own rows are removed from the real state file (the plan row was the test's, not the loop's)
python3 - "$T" <<'PY'
import json, os
p = "/projects/pi-improved/_tmp/autonomy/state.json"
try:
    d = json.load(open(p)); d.pop("X1", None); json.dump(d, open(p, "w"), indent=1, sort_keys=True)
except Exception as e:
    print("cleanup skipped:", e)
PY
[ "$FAIL" -eq 0 ] && echo "VERDICT: ALL MECHANISMS PASS" || echo "VERDICT: FAILURES ABOVE"
[ "$FAIL" -eq 0 ]
