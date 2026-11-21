#!/usr/bin/env bash
# row-paths-closes-files-at-any-heading.test.sh — the `## Files` section ends at ANY heading (spec 436's neighbour).
#
# Why: `declared()` in row-paths.py closed the section only on a level-2 heading, while intake requires the criteria to
# be repeated under `### New in this row` (pii-add-row.py:13). The level-3 heading therefore did not close the section
# and every criterion was read as a file entry. Measured live on 2026-11-21: a cycle refused four rows with
#   NOT-A-FILE grep -rqE 'free_gb|AVAIL_GB|disk_floor' crates/pii-cli/src/ crates/pii-server/src/
# (a command ending in a slash, taken for a directory) and MISSING bash scripts/checks/no-artifacts-in-crates.sh.
#
# Three assertions, because the fix can fail in two directions:
#   * the section yields ONLY its own entries (the property)
#   * the criteria do not appear in the output (the near-miss this file exists for)
#   * a spec with no level-3 heading still yields its entries (the normal path an over-eager fix would break)
set -u
cd "$(dirname "$0")/../../.." || exit 1
RP="$PWD/scripts/autonomy/row-paths.py"
OK=0
FAILS=0
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
case_absent(){
  case "$2" in *"$3"*) echo "FAIL $1"; echo "  '$2' contains '$3'" >&2; FAILS=$((FAILS+1)) ;;
               *) echo "ok $1"; OK=$((OK+1)) ;; esac
}
T=$PWD/_tmp/row-paths-heading.$$
trap 'find "$T" -depth -delete 2>/dev/null' EXIT
mkdir -p "$T"

# 1 + 2. a spec with the intake heading after ## Files
cat >"$T/s-level3.md" <<'SPEC'
# Spec: fixture
#: depends=- status=pending block=-

## Exit Criteria
- [ ] `bash b.sh`

## Files
- `a.sh`

### New in this row
- [ ] `bash b.sh`

## Guardrails
- none
SPEC
OUT=$(python3 "$RP" files "$T/s-level3.md" 2>&1)
check a-level-3-heading-closes-the-files-section "$OUT" "a.sh"
case_absent the-criteria-are-not-read-as-paths "$OUT" "bash b.sh"

# 3. the normal path: no level-3 section at all, Files is the last section
cat >"$T/s-plain.md" <<'SPEC'
# Spec: fixture
#: depends=- status=pending block=-

## Files
- `x.rs`
- `y.rs`
SPEC
OUT=$(python3 "$RP" files "$T/s-plain.md" 2>&1)
check CONTROL-a-spec-without-the-heading-still-works "$OUT" "x.rs
y.rs"

if [ "$FAILS" -eq 0 ]; then echo "RESULT: PASS ($OK assertions)"; exit 0; fi
echo "RESULT: FAIL $FAILS"
exit 1