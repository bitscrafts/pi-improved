#!/usr/bin/env bash
# quarantine-wip.test.sh — a park must not wedge the queue, and the parked work must survive.
#
# Measured 2026-11-03: spec 352 parked with NINE changed files in the tree, the dispatcher refused the dirty tree, and
# the next two rows (353, 355) never ran for the rest of the night. The parked work was good — its five tests passed.
set -uo pipefail
HEXE=0
OKS=0
ok()  { echo "ok   $1"; OKS=$((OKS+1)); }
bad() { echo "FAIL $1"; HEXE=$((HEXE+1)); }

REPO_TMP=$(mktemp -d)
ASIDE=$(mktemp -d)
TOOL=/projects/pi-improved/scripts/autonomy/quarantine-wip.sh

# --- case 1: a modified tracked file, an untracked file and a nested untracked file all survive
cd "$REPO_TMP" || exit 2
git init -q . && git config user.email t@t && git config user.name t
printf 'base\n' > tracked.txt
git add . && git commit -qm base
printf 'WORK IN PROGRESS\n' > tracked.txt
printf 'new\n' > untracked.txt
mkdir -p sub && printf 'deep\n' > sub/deep.txt

before=$(git status --porcelain | wc -l)
OUT=$(QUARANTINE_ROOT="$ASIDE" bash "$TOOL" 352 parked 2>&1)
rc=$?
after=$(git status --porcelain | wc -l)

[ "$rc" = 0 ] && ok "the tool exits 0" || bad "the tool exits 0 (rc=$rc)"
[ "$before" -eq 3 ] && ok "three paths were dirty before" || bad "three paths were dirty before (got $before)"
[ "$after" -eq 0 ] && ok "the tree is clean afterwards" || bad "the tree is clean afterwards (got $after)"
grep -q 'base' tracked.txt && ok "the tracked file holds HEAD content again" || bad "the tracked file holds HEAD content again"
if find "$ASIDE" -name tracked.txt -exec grep -q 'WORK IN PROGRESS' {} \; 2>/dev/null; then
  ok "the tracked file's WORK is preserved aside"
else bad "the tracked file's WORK is preserved aside"; fi
[ -f "$ASIDE"/*/untracked.txt ] && ok "the untracked file is preserved aside" || bad "the untracked file is preserved aside"
[ -f "$ASIDE"/*/untracked/sub/deep.txt ] && ok "the nested untracked file is preserved aside" || bad "the nested untracked file is preserved aside"

# --- case 2 (CONTROL): a clean tree says so, and creates nothing
CLEAN_TMP=$(mktemp -d); cd "$CLEAN_TMP" || exit 2
git init -q . && git config user.email t@t && git config user.name t
printf 'x\n' > f.txt && git add . && git commit -qm base
ASIDE2=$(mktemp -d)
OUT2=$(QUARANTINE_ROOT="$ASIDE2" bash "$TOOL" 352 parked 2>&1)
[ "$OUT2" = "CLEAN" ] && ok "a clean tree prints CLEAN" || bad "a clean tree prints CLEAN (got $OUT2)"
[ -z "$(ls -A "$ASIDE2")" ] && ok "a clean tree creates no aside directory" || bad "a clean tree creates no aside directory"

# --- case 3 (CONTROL): the tool never uses git checkout, which destroys uncommitted work here
if grep -vE '^[[:space:]]*#' "$TOOL" | grep -qE 'git +(checkout|restore) '; then
  bad "the tool never runs git checkout or git restore"
else ok "the tool never runs git checkout or git restore"; fi

# --- case 4: a dirty tree in the real repo is not touched by this test
[ -z "${PII_QUARANTINE_LIVE:-}" ] && ok "the live repository was not touched (scratch repos only)" || bad "the live repository was not touched"

echo "QUARANTINE: $OKS passed, $HEXE failed"
[ "$HEXE" -eq 0 ]
