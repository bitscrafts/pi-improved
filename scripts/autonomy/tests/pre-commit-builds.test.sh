#!/usr/bin/env bash
# pre-commit-builds.test.sh — the pre-commit door (spec 315).
# Drives scripts/autonomy/pre-commit-builds.sh through its live interface only: every row is a real `git archive`
# of a scratch repo and a real cargo. The scratch repos live under _tmp/ and are named by PRE_COMMIT_REPO, so this
# test never stages, commits or builds anything in the tree it lives in.
# Last line is `N ok, M FAIL`; exits 0 only if every row holds.
set -u
cd "$(dirname "$0")/../../.." || exit 1
H=$PWD/scripts/autonomy/pre-commit-builds.sh
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per row
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# hit <text> <regex>: yes when the text has a line matching the regex
hit(){ printf '%s\n' "$1" | grep -qE -- "$2" && echo yes || echo no; }
# gate <repo>: the default (gate) mode; OUT and RC are set
gate(){ OUT=$(PRE_COMMIT_REPO=$1 HEAD_BUILDS_TARGET=$TGT bash "$H" 2>&1); RC=$?; }
# fresh <name>: a copy of the template as a repo with one commit; prints its path
fresh(){
  R=$SCR/$1
  cp -R "$TPL" "$R" && git -C "$R" init -q && git -C "$R" add -A \
    && git -C "$R" -c user.name=pc -c user.email=pc@test.invalid commit -qm init || { echo "fixture failed: $1" >&2; exit 1; }
  echo "$R"
}

mkdir -p _tmp
SCR=$(mktemp -d "$PWD/_tmp/pre-commit-test.XXXXXX") || exit 1
trap 'case "$SCR" in "$PWD"/_tmp/pre-commit-test.*) rm -r -- "$SCR" ;; esac' EXIT
TGT=$SCR/target   # one compiler cache for every row: a row is seconds, not a cold build
TPL=$SCR/tpl

# the template: a workspace of app -> util, plus extra (a member app does not depend on yet), with a committed lock
mkdir -p "$TPL/app/src" "$TPL/util/src" "$TPL/extra/src"
printf '[workspace]\nresolver = "2"\nmembers = ["app", "util", "extra"]\n' >"$TPL/Cargo.toml"
printf '[package]\nname = "app"\nversion = "0.0.0"\nedition = "2021"\n\n[dependencies]\nutil = { path = "../util" }\n' >"$TPL/app/Cargo.toml"
echo 'fn main() { println!("{}", util::hello()); }' >"$TPL/app/src/main.rs"
printf '[package]\nname = "util"\nversion = "0.0.0"\nedition = "2021"\n' >"$TPL/util/Cargo.toml"
printf 'mod gone;\n\npub fn hello() -> &'"'"'static str { "hi" }\n' >"$TPL/util/src/lib.rs"
echo 'pub fn bye() {}' >"$TPL/util/src/gone.rs"
printf '[package]\nname = "extra"\nversion = "0.0.0"\nedition = "2021"\n' >"$TPL/extra/Cargo.toml"
echo 'pub fn x() {}' >"$TPL/extra/src/lib.rs"
(cd "$TPL" && cargo metadata --format-version 1 >/dev/null 2>&1) || { echo "fixture: cargo metadata failed" >&2; exit 1; }

# a-staged-broken-file-refuses
R=$(fresh broken)
sed -i 's/util::hello()/util::no_such_fn()/' "$R/app/src/main.rs"; git -C "$R" add -- app/src/main.rs
gate "$R"
check "a-staged-broken-file-refuses: exit 1" "$RC" "1"
check "a-staged-broken-file-refuses: names the error" "$(hit "$OUT" 'cannot find|error\[E')" "yes"

# CONTROL-a-staged-good-change-builds
R=$(fresh good)
sed -i 's/util::hello()/util::hello().len()/' "$R/app/src/main.rs"; git -C "$R" add -- app/src/main.rs
gate "$R"
check "CONTROL-a-staged-good-change-builds: exit 0" "$RC" "0"
check "CONTROL-a-staged-good-change-builds: says it builds" "$(hit "$OUT" 'the pending tree builds')" "yes"

# CONTROL-the-worktree-alone-is-not-read
R=$(fresh worktree-only)
sed -i 's/util::hello()/util::no_such_fn()/' "$R/app/src/main.rs"
gate "$R"
check "CONTROL-the-worktree-alone-is-not-read: exit 0" "$RC" "0"
check "CONTROL-the-worktree-alone-is-not-read: nothing staged" "$(hit "$OUT" 'nothing staged')" "yes"

# a-staged-deletion-is-applied
R=$(fresh deletion)
git -C "$R" rm -q -- util/src/gone.rs
sed -i '/^mod gone;$/d' "$R/util/src/lib.rs"; git -C "$R" add -- util/src/lib.rs
gate "$R"
check "a-staged-deletion-is-applied: exit 0" "$RC" "0"

# CONTROL-a-deletion-without-its-mod-refuses
R=$(fresh deletion-no-mod)
git -C "$R" rm -q -- util/src/gone.rs
gate "$R"
check "CONTROL-a-deletion-without-its-mod-refuses: exit 1" "$RC" "1"
check "CONTROL-a-deletion-without-its-mod-refuses: names the module" "$(hit "$OUT" 'gone')" "yes"

# a-stale-lock-refuses
R=$(fresh stale-lock)
printf 'extra = { path = "../extra" }\n' >>"$R/app/Cargo.toml"; git -C "$R" add -- app/Cargo.toml
gate "$R"
check "a-stale-lock-refuses: exit 1" "$RC" "1"
check "a-stale-lock-refuses: names the lock" "$(hit "$OUT" '--locked|lock file')" "yes"

# CONTROL-a-committed-lock-passes
R=$(fresh lock-passes)
printf 'extra = { path = "../extra" }\n' >>"$R/app/Cargo.toml"
(cd "$R" && cargo metadata --format-version 1 >/dev/null 2>&1)
git -C "$R" add -- app/Cargo.toml Cargo.lock
gate "$R"
check "CONTROL-a-committed-lock-passes: exit 0" "$RC" "0"

# CONTROL-a-clean-index-says-so
R=$(fresh clean)
gate "$R"
check "CONTROL-a-clean-index-says-so: exit 0" "$RC" "0"
check "CONTROL-a-clean-index-says-so: nothing to gate" "$(hit "$OUT" 'nothing to gate')" "yes"

# --print-names-the-warm-target: the default target, the command, and no compile
R=$(fresh print)
sed -i 's/util::hello()/util::hello().len()/' "$R/app/src/main.rs"; git -C "$R" add -- app/src/main.rs
OUT=$(env -u HEAD_BUILDS_TARGET PRE_COMMIT_REPO=$R bash "$H" --print 2>&1); RC=$?
check "--print-names-the-warm-target: exit 0" "$RC" "0"
check "--print-names-the-warm-target: names the warm target" "$(hit "$OUT" '_tmp/head-builds-target')" "yes"
check "--print-names-the-warm-target: names the check" "$(hit "$OUT" 'cargo check -q --workspace')" "yes"
check "--print-names-the-warm-target: no compile happened" "$([ -e "$R/_tmp/head-builds-target" ] && echo yes || echo no)" "no"

# an export that cannot start (no repo at PRE_COMMIT_REPO) is a loud exit 2, never a green
gate "$SCR/no-such-repo"
check "a-missing-repo-refuses-with-2: exit 2" "$RC" "2"
check "a-missing-repo-refuses-with-2: says why" "$(hit "$OUT" '^pre-commit: ')" "yes"

# the cycle calls the door before the commit, under a timeout
P=$(grep -n 'pre-commit-builds.sh' scripts/autonomy/cycle.sh | grep timeout | head -1 | cut -d: -f1)
C=$(grep -n 'git -c user.name="' scripts/autonomy/cycle.sh | head -1 | cut -d: -f1)
check "the-cycle-calls-it-before-the-commit: called under timeout, above the commit" \
  "$([ -n "$P" ] && [ -n "$C" ] && [ "$P" -lt "$C" ] && echo yes || echo no)" "yes"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
