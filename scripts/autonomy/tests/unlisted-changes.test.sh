#!/usr/bin/env bash
# unlisted-changes.test.sh — the commit step refuses while the worker's changes include a file its row did not list (spec 268).
# Exits 0 only if every assertion holds. Builds throwaway git repos under _tmp/ and runs the helper in them; never runs cycle.sh, the queue or a gate.
set -u
cd "$(dirname "$0")/../../.." || exit 1
H="$(pwd)/scripts/autonomy/unlisted-changes.sh"
CYCLE=scripts/autonomy/cycle.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# mkrepo <dir>: a repo with a committed baseline (src/, a queue HANDOFF, a QUEUE-BRIEF, a __pycache__ file, a gitignored log rule)
mkrepo(){
  mkdir -p "$1/src" "$1/scripts/autonomy/__pycache__" || return 1
  (
    cd "$1" && git init -q && git config user.email t@example.invalid && git config user.name t
    printf '%s\n' 'ignored.log' >.gitignore
    echo a >src/a.rs; echo b >src/b.rs; echo h >HANDOFF.md; echo q >QUEUE-BRIEF-x.md
    echo p >scripts/autonomy/__pycache__/x.pyc
    git add -A && git commit -qm base
  )
}
# run <repo> <paths>: the helper's output, then rc=<its exit code> on the last line
run(){ (cd "$1" && bash "$H" "$2"; echo "rc=$?"); }

T=_tmp/unlisted-changes.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT

# (a) an unlisted modified tracked file is printed and the exit is 1 (the A131 case: nodes.rs edited, not in the row)
R=$T/a; mkrepo "$R"; echo x >>"$R/src/a.rs"
check "an unlisted modified file is reported" "$(run "$R" "src/b.rs")" "$(printf 'src/a.rs\nrc=1')"

# (b) the same file, listed, is not printed and the exit is 0
check "a listed file is not" "$(run "$R" "src/a.rs")" "rc=0"

# (c) CONTROL: a listed directory covers the files beneath it
R=$T/c; mkrepo "$R"; echo x >>"$R/src/a.rs"
check "CONTROL a listed directory covers its files" "$(run "$R" "src")" "rc=0"

# (d) CONTROL: the queue's own files (HANDOFF.md, a __pycache__ file, QUEUE-BRIEF*) are not reported, modified or created
R=$T/d; mkrepo "$R"
echo x >>"$R/HANDOFF.md"; echo x >>"$R/scripts/autonomy/__pycache__/x.pyc"
echo x >>"$R/QUEUE-BRIEF-x.md"; echo y >"$R/QUEUE-BRIEF-y.md"
check "CONTROL queue-owned files are ignored" "$(run "$R" "src/a.rs")" "rc=0"

# (e) an unlisted untracked source file is printed
R=$T/e; mkrepo "$R"; echo n >"$R/src/new.rs"
check "an unlisted untracked file is reported" "$(run "$R" "src/a.rs")" "$(printf 'src/new.rs\nrc=1')"

# (f) an ignored file (matched by .gitignore) is not printed
R=$T/f; mkrepo "$R"; echo n >"$R/ignored.log"
check "an ignored file is not" "$(run "$R" "src/a.rs")" "rc=0"

# (g) an unlisted deleted tracked file is printed
R=$T/g; mkrepo "$R"; rm -f "$R/src/b.rs"
check "an unlisted deletion is reported" "$(run "$R" "src/a.rs")" "$(printf 'src/b.rs\nrc=1')"

# (h) a clean tree prints nothing and exits 0
R=$T/h; mkrepo "$R"
check "a clean tree is silent" "$(run "$R" "src/a.rs")" "rc=0"

# (i) cycle.sh calls the helper between the explicit git add and the staged-diff check, and names unlisted-changes
ADD=$(grep -n -F 'git add -- "${P[@]}"' "$CYCLE" | head -1 | cut -d: -f1)
UL=$(grep -n -F 'unlisted-changes.sh "$PATHS"' "$CYCLE" | head -1 | cut -d: -f1)
DF=$(grep -n -F 'git diff --cached --quiet' "$CYCLE" | head -1 | cut -d: -f1)
ORDER=no
if [ -n "$ADD" ] && [ -n "$UL" ] && [ -n "$DF" ] && [ "$ADD" -lt "$UL" ] && [ "$UL" -lt "$DF" ]; then ORDER=yes; fi
check "cycle.sh checks before the diff" "$ORDER" "yes"
check "cycle.sh names unlisted-changes" "$(grep -q 'unlisted-changes' "$CYCLE" && echo yes || echo no)" "yes"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
