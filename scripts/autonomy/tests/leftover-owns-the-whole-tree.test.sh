#!/usr/bin/env bash
# requires: dev-tree
# (the CONTROL runs the OLD script out of git; an export has no .git, and a control that cannot fetch the artefact it
#  exists for must be SKIPPED loudly, not dropped quietly -- the runner reports the skip and does not count it)
# leftover-owns-the-whole-tree.test.sh — a stash is tree-wide, so ownership must be tree-wide (Opus review 2026-11-01).
#
# Measured before this change: owner() counted a row the moment ONE dirty path matched, so a row naming 2 of 10
# files reported OWNER, and clean() then ran `git stash push --include-untracked` over the WHOLE tree -- taking the
# other 8 paths, which belong to whoever else is working. On 2026-11-01 that would have been the interactive edits
# in this repo. The rule now: a row owns the tree only if it names EVERY dirty path; anything else is FOREIGN and
# clean() refuses, naming what it would have swallowed.
#
# The control is the OLD script, taken from git (`git show HEAD:...`), run against the same fixture: it must report
# OWNER where the new one reports FOREIGN. A control that re-implements the old rule would only prove my memory of
# it; this drives the artefact itself.
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T="$PWD/_tmp/leftover-owns.$$"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T/repo/scripts/autonomy"
cp scripts/autonomy/leftover.sh "$T/repo/scripts/autonomy/leftover.sh"
# pinned to the commit BEFORE the fix: `git show HEAD:` would be the fixed script once this landed, and a control
# that compares the fix with itself proves nothing. If that commit ever disappears the test fails loudly.
git show 1032386:scripts/autonomy/leftover.sh > "$T/repo/scripts/autonomy/leftover-old.sh"
cd "$T/repo" || exit 1
git init -q . && git config user.email t@t && git config user.name t
printf '%s\n' a > a.txt; printf '%s\n' b > b.txt; printf '%s\n' c > c.txt
printf 'id\tspec\tbrief\ttitle\tverify\tpaths\tgate\n900\t900-spec.md\t-\tT\t-\ta.txt;b.txt\t1\n' > scripts/autonomy/plan.tsv
printf '{\n "900": {\n  "status": "needs-review",\n  "session": "sess-1"\n }\n}\n' > state.json
# the scaffolding is COMMITTED, so the only dirty paths in this fixture are the ones the cases touch -- the first
# version of this test left plan.tsv, state.json and the two scripts untracked, so "how many dirty paths?" counted
# them and every assertion after that measured the fixture instead of the behaviour.
printf '_tmp/\n' > .gitignore   # the real repo ignores _tmp/, and clean() writes its record there
git add . && git commit -qm base
export PII_PLAN="$PWD/scripts/autonomy/plan.tsv" PII_STATE="$PWD/state.json"
N=scripts/autonomy/leftover.sh
O=scripts/autonomy/leftover-old.sh

# CASE 1: the row names EVERY dirty path -> OWNER, and clean() may act
printf '%s\n' A2 > a.txt; printf '%s\n' B2 > b.txt
check "a row that names every dirty path is OWNER" "$(bash $N owner | cut -d' ' -f1)" "OWNER"

# CASE 2: two dirty paths, the row names one -> FOREIGN, and clean() must refuse
git checkout -q -- b.txt          # CASE 1 left b.txt dirty; this case must see exactly a.txt + c.txt
printf '%s\n' A3 > a.txt; printf '%s\n' C3 > c.txt
check "one unmatched dirty path makes it FOREIGN" "$(bash $N owner | cut -d' ' -f1)" "FOREIGN"
check "and the refusal counts both paths" "$(bash $N owner | cut -d' ' -f2)" "2"
check "clean refuses instead of stashing" "$(bash $N clean c1 >/dev/null 2>&1; echo rc=$?)" "rc=8"
check "the unrelated file is untouched" "$(cat c.txt)" "C3"
check "and so is the owned one" "$(cat a.txt)" "A3"
check "no stash was created" "$(git stash list | wc -l)" "0"

# CONTROL: the OLD script, same fixture, same case — it says OWNER, which is the defect this test exists for
check "CONTROL the old script calls this same tree OWNER" "$(bash $O owner | cut -d' ' -f1)" "OWNER"

# CASE 3: owned again -> clean() stashes, and the tree ends clean
git checkout -q -- c.txt; printf '%s\n' B3 > b.txt
OUT=$(bash $N clean c2 2>&1)
check "clean stashes an owned tree" "$(printf '%s' "$OUT" | tail -1 | cut -d' ' -f1)" "STASH"
check "and saved the content before stashing" "$(printf '%s' "$OUT" | grep -c '^SAVED')" "2"
check "and says whose paths it took" "$(printf '%s' "$OUT" | grep -c 'of 900')" "1"
check "the tree is clean afterwards" "$(git status --porcelain | wc -l)" "0"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
