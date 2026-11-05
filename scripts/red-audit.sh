#!/usr/bin/env bash
# red-audit.sh <row>... — for each row, which exit criteria ALREADY PASS on the current tree.
#
# `sdd`'s contract (the RED step) refuses a row if any criterion passes before the code exists, so a criterion
# that passes today is a regression guard, not a test of the new behaviour. This lists them, per row, with the
# criterion text, and writes one line per row to _tmp/red-audit.tsv: <row>\t<criteria>\t<already-passing>\t<cmds>.
# Read-only: it runs the criteria the spec names and changes nothing.
cd /projects/pi-improved || exit 1
export PATH=/root/.local/bin:/usr/local/cargo/bin:$PATH
OUT=_tmp/red-audit.tsv; : > "$OUT"
for r in "$@"; do
  RES=$(timeout 540 python3 scripts/autonomy/row-run-criteria.py "$r" 2>&1)
  TOTAL=$(printf '%s\n' "$RES" | grep -cE '^\s+(PASS|FAIL)')
  PASSING=$(printf '%s\n' "$RES" | grep -E '^\s+PASS' | sed -E 's/^\s+PASS rc=0\s+//' | cut -c1-110)
  N=$(printf '%s\n' "$PASSING" | grep -c .)
  echo "== row $r: $N of $TOTAL criteria already pass"
  printf '%s\n' "$PASSING" | sed 's/^/     passes: /'
  printf '%s\t%s\t%s\t%s\n' "$r" "$TOTAL" "$N" "$(printf '%s' "$PASSING" | tr '\n' ';')" >> "$OUT"
done
echo; echo "written: $OUT"
