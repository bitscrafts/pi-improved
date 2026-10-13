#!/usr/bin/env bash
# Harness for spec 153: a selftest run must leave the operator's real records byte-identical.
# Usage: operator-records-untouched.sh   (prints UNTOUCHED and exits 0 only if every hash held)
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 2
d=_tmp/autonomy
files=("$d/control" "$d/questions.md" "$d/notes.md" "$d/state.json" "$d/operator-acts.log" "$d/last-reported")
snap() { for f in "${files[@]}"; do if [ -e "$f" ]; then sha256sum "$f"; else echo "absent $f"; fi; done; }
before="$(snap)"
bash scripts/autonomy/selftest.sh >/dev/null 2>&1
rc=$?
after="$(snap)"
if [ "$before" != "$after" ]; then echo "CHANGED"; diff <(echo "$before") <(echo "$after"); exit 1; fi
[ "$rc" -eq 0 ] || { echo "selftest failed (rc=$rc)"; exit 1; }
echo UNTOUCHED
