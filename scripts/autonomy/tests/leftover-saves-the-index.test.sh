#!/usr/bin/env bash
# The tree's content must survive in a place that is not git's stash stack.
#
# Measured defect (2026-11-02): `save` captured `git diff` (unstaged tracked changes) and untracked files, and
# NOTHING ELSE — while `git stash push` also carries the INDEX. A person who stages an edit and walks away had
# that edit recorded by nothing but a stash commit: `save` wrote no file for it and `clean` never called `save`
# at all, so the documented promise ("the content must stay recoverable") rested on a single mutable ref that
# `git stash drop` removes in one word.
#
# Three assertions, each with its control:
#   1. a STAGED-ONLY change is written to <cid>-staged.patch   (control: the same edit unstaged goes to -tracked.patch)
#   2. clean() writes the save files BEFORE it stashes         (control: a later `git stash drop` loses nothing)
#   3. clean() refuses when the save fails                    (control: it stashes when the save succeeds)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../../.." && pwd)
L=${PII_LEFTOVER_SH:-$REPO/scripts/autonomy/leftover.sh}   # a seam, so the control can run the OLD script
pass=0
fail=0
ok(){ pass=$((pass+1)); echo "ok   $1"; }
no(){ fail=$((fail+1)); echo "FAIL $1"; }

mk(){ # mk <dir>: a repo with one committed file and a plan/state naming nothing
  local d=$1
  mkdir -p "$d/scripts/autonomy" "$d/_tmp/autonomy"
  git -C "$d" init -q 2>/dev/null
  git -C "$d" config user.email t@t; git -C "$d" config user.name t
  printf 'one\nsecond\n' >"$d/f.txt"; printf 'gone\n' >"$d/g.txt"
  printf '_tmp/\n' >"$d/.gitignore"   # the real repo ignores _tmp/, so leftovers are not tree noise
  git -C "$d" add . ; git -C "$d" commit -qm init
  cp "$L" "$d/scripts/autonomy/leftover.sh"
  printf '#id\trow\tplan\tstatus\towner\tpaths\tnote\n' >"$d/scripts/autonomy/plan.tsv"
  printf '{}\n' >"$d/_tmp/autonomy/state.json"
  # the fixture's own scaffolding is COMMITTED: otherwise the copied script and these two files are dirty and the
  # ownership rule (correctly) refuses, which would test the refusal instead of the save.
  git -C "$d" add -A; git -C "$d" commit -qm scaffold
}

W=$(mktemp -d); trap 'rm -rf "$W"' EXIT

# --- 1. a staged-only change is saved as a staged patch -------------------------------------------------
D=$W/staged; mk "$D"
printf 'one\nSECOND\n' >"$D/f.txt"; git -C "$D" add f.txt            # staged, working tree clean vs index
out=$(cd "$D" && bash scripts/autonomy/leftover.sh save "$D/left" cid1 2>&1)
if [ -f "$D/left/cid1-staged.patch" ] && grep -q 'SECOND' "$D/left/cid1-staged.patch"; then
  ok "a staged-only change is saved to -staged.patch"
else no "a staged-only change is saved to -staged.patch (got: $(printf '%s' "$out" | tr '\n' ' '))"; fi
if [ -f "$D/left/cid1-paths.txt" ] && [ ! -f "$D/left/cid1-tracked.patch" ]; then
  ok "  and the unstaged patch is not invented for it"
else no "  and the unstaged patch is not invented for it (save produced: $(ls "$D/left" 2>/dev/null | tr '\n' ' '))"; fi

# control: the same edit UNSTAGED lands in -tracked.patch, not -staged.patch
D2=$W/unstaged; mk "$D2"
printf 'one\nSECOND\n' >"$D2/f.txt"
(cd "$D2" && bash scripts/autonomy/leftover.sh save "$D2/left" cid2 >/dev/null 2>&1)
if [ -f "$D2/left/cid2-tracked.patch" ] && [ ! -f "$D2/left/cid2-staged.patch" ]; then
  ok "control: an unstaged change is saved as -tracked.patch"
else no "control: an unstaged change is saved as -tracked.patch"; fi

# --- 2. clean() saves BEFORE it stashes, so dropping the stash loses nothing ----------------------------
D3=$W/clean; mk "$D3"
ROW='r1'; printf '#id\trow\tplan\tstatus\towner\tpaths\tnote\n%s\tone\tx\tpending\tme\tf.txt\t\n' "$ROW" \
  >"$D3/scripts/autonomy/plan.tsv"
printf '{\n "r1": {\n  "session": "s1",\n  "status": "pending"\n }\n}\n' >"$D3/_tmp/autonomy/state.json"
git -C "$D3" add -A; git -C "$D3" commit -qm "the row and its state"
printf 'one\nSECOND\n' >"$D3/f.txt"; git -C "$D3" add f.txt       # staged: the case that used to vanish
out=$(cd "$D3" && PII_PLAN="$D3/scripts/autonomy/plan.tsv" PII_STATE="$D3/_tmp/autonomy/state.json" \
      PII_LEFTOVER_DIR="$D3/_tmp/autonomy/leftovers" bash scripts/autonomy/leftover.sh clean r1 2>&1)
stashed=0; [ -z "$(git -C "$D3" status --porcelain)" ] && stashed=1
if [ "$stashed" = 1 ]; then ok "clean() stashed the owned tree"; else no "clean() stashed the owned tree ($out)"; fi
# drop the stash outright: the person's staged edit must still be on disk in the leftovers dir
git -C "$D3" stash drop -q 2>/dev/null || true
if grep -rq 'SECOND' "$D3/_tmp/autonomy/leftovers" 2>/dev/null; then
  ok "the content survives dropping the stash (it is in the leftovers dir)"
else no "the content survives dropping the stash (leftovers dir holds: $(ls "$D3/_tmp/autonomy/leftovers" 2>/dev/null | tr '\n' ' '))"; fi

# control: nothing was stashed that is not also saved — the paths file names exactly what was stashed
if [ -s "$D3/_tmp/autonomy/leftovers/r1-paths.txt" ]; then
  ok "control: the paths file lists what the stash carried"
else no "control: the paths file lists what the stash carried"; fi

echo "RESULT: $pass ok, $fail FAIL"
[ "$fail" -eq 0 ]
