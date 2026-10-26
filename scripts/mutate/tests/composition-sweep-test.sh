#!/usr/bin/env bash
# Spec 286 R3: the sweep tests itself. It runs composition.sh on a three-row fixture table over a standalone fixture
# crate and asserts the three outcomes (RED, SURVIVED, NO-APPLY), the non-zero exit, a byte-identical fixture, and a
# CONTROL run with the SURVIVED row removed that exits 0.
# Usage: composition-sweep-test.sh   (prints PASS: n FAIL: n as its last line)
root="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$root" || exit 2
fix=scripts/mutate/tests/fixture
work=_tmp/composition-sweep-fixture
pass=0
fail=0
check() {
	if [ "$2" = "ok" ]; then
		pass=$((pass + 1))
	else
		fail=$((fail + 1))
		echo "not ok: $1"
	fi
}
fixture_digest() { find "$fix" -type f | sort | xargs sha256sum; }

mkdir -p "$work"
cat >"$work/rows.py" <<'PY'
f1 = dict(id="f1", file="src/lib.rs", edits=[("x * 2", "x * 3")], test="", kind="required")
f2 = dict(id="f2", file="src/lib.rs", edits=[("x * 2", "x + x")], test="", kind="required")
f3 = dict(id="f3", file="src/lib.rs", edits=[("x * 9", "x * 8")], test="", kind="probe")
ROWS = [f1, f2, f3]
PY
cat >"$work/control.py" <<'PY'
f1 = dict(id="f1", file="src/lib.rs", edits=[("x * 2", "x * 3")], test="", kind="required")
f3 = dict(id="f3", file="src/lib.rs", edits=[("x * 9", "x * 8")], test="", kind="probe")
ROWS = [f1, f3]
PY

before="$(fixture_digest)"
out="$work/out.txt"
CARGO_TARGET_DIR="$root/$work/target" bash scripts/mutate/composition.sh --root "$fix" --rows "$work/rows.py" \
	--scratch "$work/scratch" >"$out" 2>&1
rc=$?
after="$(fixture_digest)"

check "the fixture sweep exits non-zero" "$([ "$rc" -ne 0 ] && echo ok)"
check "f1 is RED" "$(grep -cE '^\| f1 \|.*\| RED \|$' "$out" | grep -qx 1 && echo ok)"
check "f2 is SURVIVED" "$(grep -cE '^\| f2 \|.*\| SURVIVED \|$' "$out" | grep -qx 1 && echo ok)"
check "f3 is NO-APPLY" "$(grep -cE '^\| f3 \|.*\| NO-APPLY \|$' "$out" | grep -qx 1 && echo ok)"
check "the sweep reports exactly three rows" "$(grep -cE '^\| f[0-9] \|' "$out" | grep -qx 3 && echo ok)"
check "the required count is 1/2" "$(grep -q '^MUTATION-SWEEP: required 1/2 probes 1$' "$out" && echo ok)"
check "the fixture is byte-identical" "$([ "$before" = "$after" ] && echo ok)"

CARGO_TARGET_DIR="$root/$work/target" bash scripts/mutate/composition.sh --root "$fix" --rows "$work/control.py" \
	--scratch "$work/scratch" >"$work/control-out.txt" 2>&1
control_rc=$?
check "CONTROL: without the SURVIVED row the sweep exits 0" "$([ "$control_rc" -eq 0 ] && echo ok)"

echo "PASS: $pass FAIL: $fail"
[ "$fail" -eq 0 ]
