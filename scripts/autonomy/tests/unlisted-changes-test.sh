#!/usr/bin/env bash
# unlisted-changes-test.sh — the unlisted-change check reads every shape git status prints (spec 277).
# One scratch repo per row under _tmp/, the real scripts/autonomy/unlisted-changes.sh run in it. Prints PASS/FAIL per row
# and a final RESULT line; exits non-zero on any failure. Never runs cycle.sh, the queue or a gate; never touches the tree's index.
set -u
cd "$(dirname "$0")/../../.." || exit 1
ROOT=$(pwd)
H="$ROOT/scripts/autonomy/unlisted-changes.sh"
T="$ROOT/_tmp/unlisted-changes-test.$$"
mkdir -p "$T" || exit 1
FAILS=0
# mkrepo <dir>: a repo with a committed baseline (src/keep.rs, src/a.rs, src/b.rs, "with space.rs")
mkrepo(){
  mkdir -p "$1/src" || return 1
  (
    cd "$1" && git init -q && git config user.email t@example.invalid && git config user.name t
    echo k >src/keep.rs; echo a >src/a.rs; echo b >src/b.rs; echo s >"with space.rs"
    git add -A && git commit -qm base
  )
}
# run <row> <repo> <paths>: the real helper, run in <repo>; its stdout goes to $T/<row>.out and its exit code to RC
run(){ (cd "$2" && bash "$H" "$3" >"$T/$1.out" 2>&1); RC=$?; }
# verdict <row> <want-rc> <want-output>: the last run against the row's expectation (the output is one bare path per line)
verdict(){
  local GOT; GOT=$(cat "$T/$1.out")
  if [ "$RC" -eq "$2" ] && [ "$GOT" = "$3" ]; then echo "PASS $1"
  else echo "FAIL $1: rc=$RC want $2; output '$GOT' want '$3'"; FAILS=$((FAILS+1)); fi
}

# (a) the only change is a rename, the row lists another file: refused, and the NEW path is named
R=$T/a; mkrepo "$R" && (cd "$R" && git mv src/a.rs src/new.rs)
run an-unlisted-rename-is-caught "$R" "src/keep.rs"; verdict an-unlisted-rename-is-caught 1 "src/new.rs"

# (b) CONTROL: the same rename, the row lists the new path: not refused (the old path is not a change the row must list)
R=$T/b; mkrepo "$R" && (cd "$R" && git mv src/a.rs src/new.rs)
run CONTROL-a-listed-rename-is-not-refused "$R" "src/new.rs"; verdict CONTROL-a-listed-rename-is-not-refused 0 ""

# (c) a modified path with a space, listed: not refused (git quotes it on the line form; -z does not)
R=$T/c; mkrepo "$R" && echo changed >>"$R/with space.rs"
run a-quoted-path-with-a-space-is-listed "$R" "with space.rs"; verdict a-quoted-path-with-a-space-is-listed 0 ""

# (d) the same path unlisted: refused, and the output is the bare path, without quotes
R=$T/d; mkrepo "$R" && echo changed >>"$R/with space.rs"
run an-unlisted-path-with-a-space-is-caught "$R" "src/keep.rs"; verdict an-unlisted-path-with-a-space-is-caught 1 "with space.rs"

# (e) CONTROL: a plain unlisted modification still refuses
R=$T/e; mkrepo "$R" && echo changed >>"$R/src/a.rs"
run CONTROL-an-unlisted-modification-still-refuses "$R" "src/keep.rs"; verdict CONTROL-an-unlisted-modification-still-refuses 1 "src/a.rs"

# (f) a modification under a listed directory is covered by it
R=$T/f; mkrepo "$R" && echo changed >>"$R/src/a.rs"
run a-listed-directory-covers-its-subtree "$R" "src"; verdict a-listed-directory-covers-its-subtree 0 ""

# (g) a deletion the row does not list is checked
R=$T/g; mkrepo "$R" && rm "$R/src/b.rs"
run a-deleted-file-is-checked "$R" "src/keep.rs"; verdict a-deleted-file-is-checked 1 "src/b.rs"

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS"; else echo "RESULT: FAIL $FAILS"; fi
[ "$FAILS" -eq 0 ]
