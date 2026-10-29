#!/usr/bin/env bash
# gate-touched-crates.test.sh — the default gate's touched-crate test step (spec 314).
# Drives scripts/gate-touched-crates.sh through its live interface only; the scratch git repos live under _tmp/.
# Runs no cargo: every row is --print, a no-git export, or a --run on a clean tree.
# Last line is `N ok, M FAIL`; exits 0 only if every row holds.
set -u
cd "$(dirname "$0")/../../.." || exit 1
H=scripts/gate-touched-crates.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per row
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# print_of <paths...>: the --print command, stdout only
print_of(){ bash "$H" --print "$@" 2>/dev/null; }
# pkgs: the package names named by a --print command on stdin, space-separated
pkgs(){ grep -o -- '-p [^ ]*' | sed 's/^-p //' | tr '\n' ' ' | sed 's/ $//'; }
# names_pkg <space list> <name>: yes when the name is one member of the list
names_pkg(){ case " $1 " in *" $2 "*) echo yes ;; *) echo no ;; esac; }
# make_repo <dir>: a git repo with one crate, pii-graph, committed
make_repo(){
  git -C "$1" init -q
  mkdir -p "$1/crates/pii-graph/src"
  printf '[package]\nname = "pii-graph"\nversion = "0.0.0"\n' >"$1/crates/pii-graph/Cargo.toml"
  echo 'pub fn f() {}' >"$1/crates/pii-graph/src/lib.rs"
  git -C "$1" add -A && git -C "$1" -c user.name=gate -c user.email=gate@test.invalid commit -qm init
}

mkdir -p _tmp
SCR=$(mktemp -d "$PWD/_tmp/gate-touched.XXXXXX")
trap 'case "$SCR" in "$PWD"/_tmp/gate-touched.*) rm -r -- "$SCR" ;; esac' EXIT
DIRTY="$SCR/dirty"; CLEAN="$SCR/clean"; NOGIT="$SCR/nogit"
mkdir -p "$DIRTY" "$CLEAN" "$NOGIT"
make_repo "$DIRTY"; make_repo "$CLEAN"
echo '// touched' >>"$DIRTY/crates/pii-graph/src/lib.rs"

# the mapping, read from paths passed explicitly
check "a crate source maps to its package" "$(print_of crates/pii-graph/src/sched/state.rs | pkgs)" "pii-graph"
check "a crate test file maps too" "$(print_of crates/pii-graph/tests/loops.rs | pkgs)" "pii-graph"
check "two files in one crate name it once" "$(print_of crates/pii-graph/src/sched/state.rs crates/pii-graph/tests/loops.rs | pkgs)" "pii-graph"
check "the root member maps" "$(print_of xtask/src/main.rs | pkgs)" "xtask"
check "docs and scripts name no crate" "$(print_of _docs/05-operations/open-items.md scripts/autonomy/plan.tsv)" ""
check "docs and scripts exit 0" "$(bash "$H" --print _docs/05-operations/open-items.md scripts/autonomy/plan.tsv >/dev/null 2>&1; echo $?)" "0"
check "docs and scripts say no crate touched" "$(bash "$H" --print _docs/05-operations/open-items.md scripts/autonomy/plan.tsv 2>&1 >/dev/null | grep -c 'no crate touched')" "1"
MANIFEST=$(print_of Cargo.toml | pkgs)
check "the manifest names pii-graph" "$(names_pkg "$MANIFEST" pii-graph)" "yes"
check "the manifest names xtask" "$(names_pkg "$MANIFEST" xtask)" "yes"
check "the manifest names every member" "$(echo "$MANIFEST" | wc -w | tr -d ' ')" "$(( $(ls -d crates/*/Cargo.toml | wc -l) + 1 ))"
check "an unknown crate path refuses with exit 2" "$(bash "$H" --print crates/no-such-crate/src/lib.rs >/dev/null 2>&1; echo $?)" "2"
check "an unknown crate path names itself" "$(bash "$H" --print crates/no-such-crate/src/lib.rs 2>&1 >/dev/null | grep -c 'crates/no-such-crate/src/lib.rs names no workspace package')" "1"

# the list is read from git, not from the argument list
check "the live git path is what is read" "$(GATE_REPO=$DIRTY bash "$H" --print | pkgs)" "pii-graph"
check "CONTROL: a clean tree names nothing" "$(GATE_REPO=$CLEAN bash "$H" --print 2>/dev/null)" ""
check "CONTROL: a clean tree says no crate touched" "$(GATE_REPO=$CLEAN bash "$H" --print 2>&1 >/dev/null)" "gate-touched-crates: no crate touched"
check "a tree that is not a git repo says so and exits 0" "$(GATE_REPO=$NOGIT bash "$H" --print 2>&1 >/dev/null; echo "rc=$?")" "gate-touched-crates: no git repository — nothing to test here
rc=0"

# the step itself, extracted from the quick list at run time
LINE=$(grep '^test (touched crates)|' gate-steps-quick.txt)
STEP=${LINE#*|}
check "CONTROL: the step line runs the helper with --run" "$(case "$STEP" in *"scripts/gate-touched-crates.sh --run"*) echo yes ;; *) echo no ;; esac)" "yes"
check "CONTROL: the step line, run on a clean tree, names nothing" "$(GATE_REPO=$CLEAN bash -c "$STEP" 2>&1; echo "rc=$?")" "gate-touched-crates: no crate touched
rc=0"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
