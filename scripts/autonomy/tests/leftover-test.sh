#!/usr/bin/env bash
# leftover-test.sh — a dirty tree is saved, attributed and cleaned by leftover.sh, never inherited (spec 311).
# Each row builds its OWN git fixture under _tmp/ (never /tmp) and runs the real leftover.sh inside it, with a
# fixture plan.tsv and state.json. Prints PASS/FAIL per row, then RESULT. Exits non-zero on any failure.
# Never runs cycle.sh, a gate or a worker; the live worktree, plan.tsv and state.json are never read or written.
set -u
cd "$(dirname "$0")/../../.." || exit 1
ROOT=$(pwd)
LO="$ROOT/scripts/autonomy/leftover.sh"
FAILS=0

# scratch: a new empty directory under _tmp/ for one row
scratch(){ mkdir -p "$ROOT/_tmp" && mktemp -d "$ROOT/_tmp/leftover-XXXXXX"; }
# fixture <repo>: a git repository with one committed file a.txt (its own .git, so git inside it is the fixture's)
fixture(){
  mkdir -p "$1" && (cd "$1" && git init -q && git config user.email t@example.invalid && git config user.name t \
    && echo base >a.txt && git add a.txt && git commit -qm base)
}
# lo <repo> <verb> [args]: run leftover.sh with the repository as the working directory
lo(){ local d=$1; shift; (cd "$d" && bash "$LO" "$@"); }
# own <repo> <plan> <state>: run `leftover.sh owner` against fixture plan and state files
own(){ (cd "$1" && PII_PLAN="$2" PII_STATE="$3" bash "$LO" owner); }
# verdict <name> <ok: yes|no> <why>: one PASS or FAIL line per row
verdict(){
  if [ "$2" = yes ]; then echo "PASS $1"; else echo "FAIL $1: $3"; FAILS=$((FAILS+1)); fi
}
# state.json in state.py's own layout (indent=1, sorted keys): one key per line, so the reader sees each row's fields
STATE_ONE_SESSION='{\n "R1": {\n  "session": "s1",\n  "status": "needs-review"\n }\n}\n'
STATE_NO_SESSION='{\n "R1": {\n  "status": "needs-review"\n }\n}\n'
STATE_TWO_SESSIONS='{\n "R1": {\n  "session": "s1",\n  "status": "needs-review"\n },\n "R2": {\n  "session": "s2",\n  "status": "needs-review"\n }\n}\n'

# 1. a modified tracked file is saved as a patch, and clean leaves the tree empty
D=$(scratch); fixture "$D/repo"; echo changed >>"$D/repo/a.txt"
lo "$D/repo" save "$D/leftovers" 311a >"$D/save.out" 2>&1; SRC=$?
lo "$D/repo" clean 311a >"$D/clean.out" 2>&1; CRC=$?
PORC=$(cd "$D/repo" && git status --porcelain)
if [ "$SRC" = 0 ] && [ -s "$D/leftovers/311a-tracked.patch" ] && [ "$CRC" = 0 ] && [ -z "$PORC" ]; then
  verdict a-tracked-change-is-saved-and-cleaned yes
else
  verdict a-tracked-change-is-saved-and-cleaned no "save rc=$SRC clean rc=$CRC porcelain='$PORC'; out: $(cat "$D/save.out" "$D/clean.out")"
fi

# CONTROL: an untracked file is saved by content, cleaned away, and comes back byte-identical after stash pop
D=$(scratch); fixture "$D/repo"; printf 'new bytes\n' >"$D/repo/new.txt"
SUM=$(sha256sum "$D/repo/new.txt" | cut -d' ' -f1)
lo "$D/repo" save "$D/leftovers" 311b >/dev/null 2>&1; SRC=$?
lo "$D/repo" clean 311b >/dev/null 2>&1; CRC=$?
GONE=no; [ ! -e "$D/repo/new.txt" ] && GONE=yes
(cd "$D/repo" && git stash pop >/dev/null 2>&1)
SUM2=$(sha256sum "$D/repo/new.txt" 2>/dev/null | cut -d' ' -f1)
if [ "$SRC" = 0 ] && [ "$CRC" = 0 ] && [ "$GONE" = yes ] && [ -s "$D/leftovers/311b-untracked.tar" ] && [ "$SUM" = "$SUM2" ]; then
  verdict CONTROL-an-untracked-file-keeps-its-content yes
else
  verdict CONTROL-an-untracked-file-keeps-its-content no "save=$SRC clean=$CRC gone=$GONE sum '$SUM' vs '$SUM2'"
fi

# 2. a staged rename lists BOTH paths, so the list never names a path that no longer exists
D=$(scratch); fixture "$D/repo"; (cd "$D/repo" && git mv a.txt b.txt)
lo "$D/repo" save "$D/leftovers" 311c >/dev/null 2>&1; SRC=$?
PATHS="$D/leftovers/311c-paths.txt"
if [ "$SRC" = 0 ] && grep -qx 'a.txt' "$PATHS" 2>/dev/null && grep -qx 'b.txt' "$PATHS" 2>/dev/null; then
  verdict a-rename-is-listed-by-both-paths yes
else
  verdict a-rename-is-listed-by-both-paths no "save rc=$SRC; paths.txt: $(cat "$PATHS" 2>/dev/null | tr '\n' ' ')"
fi

# CONTROL: a clean tree is a no-op: owner says FOREIGN 0, save writes no patch, clean creates no stash
D=$(scratch); fixture "$D/repo"
printf '%b' 'R1\tS.md\t-\tfixture row\ttrue\ta.txt\t0\n' >"$D/plan.tsv"; printf '%b' "$STATE_ONE_SESSION" >"$D/state.json"
OUT=$(own "$D/repo" "$D/plan.tsv" "$D/state.json"); ORC=$?
lo "$D/repo" save "$D/leftovers" 311d >/dev/null 2>&1
lo "$D/repo" clean 311d >/dev/null 2>&1; CRC=$?
STASHES=$(cd "$D/repo" && git stash list)
if [ "$OUT" = "FOREIGN 0" ] && [ "$ORC" = 7 ] && [ ! -e "$D/leftovers/311d-tracked.patch" ] && [ "$CRC" = 0 ] && [ -z "$STASHES" ]; then
  verdict CONTROL-a-clean-tree-is-a-no-op yes
else
  verdict CONTROL-a-clean-tree-is-a-no-op no "owner='$OUT' rc=$ORC clean=$CRC stashes='$STASHES'"
fi

# 3. the loop's own work: one row, with a session, not done, naming the dirty path in column 6
D=$(scratch); fixture "$D/repo"; echo changed >>"$D/repo/a.txt"
printf '%b' 'R1\tS.md\t-\tfixture row\ttrue\ta.txt\t0\n' >"$D/plan.tsv"; printf '%b' "$STATE_ONE_SESSION" >"$D/state.json"
OUT=$(own "$D/repo" "$D/plan.tsv" "$D/state.json"); ORC=$?
if [ "$OUT" = "OWNER R1" ] && [ "$ORC" = 0 ]; then verdict the-loops-own-work-is-attributed yes
else verdict the-loops-own-work-is-attributed no "owner='$OUT' rc=$ORC, want 'OWNER R1' rc 0"; fi

# 4. the near miss: the same path, but the row holds no session, so nobody can say it is the loop's
D=$(scratch); fixture "$D/repo"; echo changed >>"$D/repo/a.txt"
printf '%b' 'R1\tS.md\t-\tfixture row\ttrue\ta.txt\t0\n' >"$D/plan.tsv"; printf '%b' "$STATE_NO_SESSION" >"$D/state.json"
OUT=$(own "$D/repo" "$D/plan.tsv" "$D/state.json"); ORC=$?
if [ "$OUT" = "FOREIGN 1" ] && [ "$ORC" = 7 ]; then verdict a-row-without-a-session-is-foreign yes
else verdict a-row-without-a-session-is-foreign no "owner='$OUT' rc=$ORC, want 'FOREIGN 1' rc 7"; fi

# 5. ambiguity is not a guess: two open rows, both with sessions, both name the same dirty path
D=$(scratch); fixture "$D/repo"; echo changed >>"$D/repo/a.txt"
printf '%b' 'R1\tS.md\t-\tfirst\ttrue\ta.txt\t0\nR2\tS.md\t-\tsecond\ttrue\ta.txt\t0\n' >"$D/plan.tsv"
printf '%b' "$STATE_TWO_SESSIONS" >"$D/state.json"
OUT=$(own "$D/repo" "$D/plan.tsv" "$D/state.json"); ORC=$?
if [ "$OUT" = "FOREIGN 1" ] && [ "$ORC" = 7 ]; then verdict two-claimed-rows-are-foreign yes
else verdict two-claimed-rows-are-foreign no "owner='$OUT' rc=$ORC, want 'FOREIGN 1' rc 7"; fi

# 6. a clean that cannot clean refuses: a held .git/index.lock makes the stash fail; nothing is destroyed
D=$(scratch); fixture "$D/repo"; echo changed >>"$D/repo/a.txt"; touch "$D/repo/.git/index.lock"
lo "$D/repo" clean 311f >"$D/clean.out" 2>&1; CRC=$?
STILL=$(cd "$D/repo" && git status --porcelain)
if [ "$CRC" = 8 ] && grep -q '^NOT-CLEANED' "$D/clean.out" && [ -n "$STILL" ] && [ "$(cat "$D/repo/a.txt")" = "$(printf 'base\nchanged')" ]; then
  verdict a-clean-step-that-cannot-clean-refuses yes
else
  verdict a-clean-step-that-cannot-clean-refuses no "clean rc=$CRC, want 8 and NOT-CLEANED; out: $(cat "$D/clean.out")"
fi

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS"; exit 0; fi
echo "RESULT: FAIL $FAILS"
exit 1
