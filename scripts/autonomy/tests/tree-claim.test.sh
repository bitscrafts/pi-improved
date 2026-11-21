#!/usr/bin/env bash
# tree-claim.test.sh — a row that CLAIMS the tree owns it, with no session (spec 433).
#
# Why: owner() proved ownership with the row's `session`, and the cycle writes session="" whenever the worker lane
# refuses - every row on a machine without podman. A row implemented by hand could never own its tree, and clean
# stashed it (rows 414-418, 429).
#
# The fixture is written the way the working probe wrote it (measured 2026-11-21T06:0xZ: `STATE_ROWS='A1|pending||claimed'`
# -> `clm=claimed` -> `OWNER A1`). A parameterised version of this file kept failing for a reason that was never
# established, so the fixture is literal: a test whose setup is cleverer than its assertion tests the setup.
set -u
cd "$(dirname "$0")/../../.." || exit 1
LO="$PWD/scripts/autonomy/leftover.sh"
OK=0
FAILS=0
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
T=$PWD/_tmp/tree-claim.$$
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
mkdir -p "$T/repo/src"
cd "$T/repo" || exit 1
git init -q . && git config user.email t@t && git config user.name t || exit 1
printf 'a\n' >src/a.sh
printf 'b\n' >src/b.sh
git add -A >/dev/null && git commit -qm base || exit 1
printf 'x\n' >>src/a.sh
printf 'y\n' >>src/b.sh
printf 'A1\tS.md\t-\ttitle\ttrue\tsrc/a.sh;src/b.sh\t1\n' >"$T/plan.tsv"

# 1. the property: claimed, no session anywhere in the state
printf '{\n "A1": {\n  "status": "pending",\n  "tree": "claimed"\n }\n}\n' >"$T/state.json"
GOT=$(PII_PLAN="$T/plan.tsv" PII_STATE="$T/state.json" bash "$LO" owner 2>/dev/null)
check a-claim-owns-the-tree-without-a-session "$GOT" "OWNER A1"

# 2. the control: identical tree, no claim, no session -> FOREIGN and exit 7
printf '{\n "A1": {\n  "status": "pending"\n }\n}\n' >"$T/state.json"
GOT=$(PII_PLAN="$T/plan.tsv" PII_STATE="$T/state.json" bash "$LO" owner 2>/dev/null); RC=$?
check CONTROL-no-claim-is-foreign "$GOT" "FOREIGN 2"
check CONTROL-foreign-exits-7 "$RC" "7"

# 3. a claim does not swallow a path the row does not name: `clean` must never stash other people's work
printf 'z\n' >src/c.sh
printf '{\n "A1": {\n  "status": "pending",\n  "tree": "claimed"\n }\n}\n' >"$T/state.json"
GOT=$(PII_PLAN="$T/plan.tsv" PII_STATE="$T/state.json" bash "$LO" owner 2>/dev/null)
check a-claim-does-not-swallow-an-unnamed-path "$GOT" "FOREIGN 3"
rm -f src/c.sh

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS ($OK assertions)"; exit 0; fi
echo "RESULT: FAIL $FAILS"
exit 1