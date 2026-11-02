#!/usr/bin/env bash
# fix-row.sh <id> — make a spec's `## Files` and `## Exit Criteria` say exactly what its plan row says.
#
# WHY THIS EXISTS. The queue refuses a row whose spec disagrees with it (`NO-FILES`, `NOT-A-CRITERION`, `COUNT`,
# `DIFF`), which is the right rule — a row is a contract and the spec is its text. But at 03:00 the disagreement is
# almost always mechanical: a criterion written with a trailing arrow, a file list missing the test file. Making a
# model reword a spec to satisfy a parser wastes the night, and a model that "fixes" the criteria could weaken them.
# This script never invents anything: the ROW is authoritative, and it rewrites the two machine-consumed blocks of
# the spec to match it, byte for byte. Then it re-runs the repo's own two checkers and exits on their verdict.
#
# Usage: fix-row.sh <row-id>          rewrite the spec to match the row, then verify
#        fix-row.sh --check <id>     verify only, change nothing
# Env: PII_PLAN, PII_SPECS (as the checkers read them).
set -uo pipefail
cd /projects/pi-improved || exit 1
PLAN=${PII_PLAN:-scripts/autonomy/plan.tsv}
SPECS=${PII_SPECS:-specs}

mode=fix
if [ "${1:-}" = "--check" ]; then mode=check; shift; fi
id=${1:?usage: fix-row.sh [--check] <row-id>}

spec=$(awk -F'\t' -v id="$id" '$1==id {print $2; exit}' "$PLAN")
[ -n "$spec" ] || { echo "fix-row: no row $id in $PLAN"; exit 2; }
paths=$(awk -F'\t' -v id="$id" '$1==id {print $6; exit}' "$PLAN")
verify=$(awk -F'\t' -v id="$id" '$1==id {print $5; exit}' "$PLAN")

if [ "$mode" = fix ]; then
  python3 - "$SPECS/$spec" "$paths" "$verify" "$id" <<'PY'
import sys
spec, paths, verify, row = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
src = open(spec, encoding="utf-8").read()

def replace_block(text, header, body, next_headers):
    """Replace `## header` .. up to the next header in next_headers (or EOF) with header + body."""
    start = text.find(header)
    if start < 0:
        return text.rstrip() + "\n\n" + header + "\n\n" + body + "\n"
    ends = [text.find(h, start + len(header)) for h in next_headers]
    ends = [e for e in ends if e >= 0]
    end = min(ends) if ends else len(text)
    return text[:start] + header + "\n\n" + body + "\n\n" + text[end:]

files = [p for p in paths.split(";") if p.strip()]
crit = [c for c in verify.split(";") if c.strip()]
out = replace_block(src, "## Exit Criteria", "\n".join(f"- [ ] `{c}`" for c in crit),
                    ["## Guardrails", "## Files"])
out = replace_block(out, "## Files", "\n".join(f"- `{p}`" for p in files), ["## Exit Criteria"])
open(spec, "w", encoding="utf-8").write(out.rstrip() + "\n")
print(f"fix-row: {spec} now states {len(files)} file(s) and {len(crit)} criterion/criteria from row {row}")
PY
fi

rc=0
python3 scripts/autonomy/row-paths.py check "$id" || rc=1
python3 scripts/autonomy/row-verify.py check "$id" || rc=1
exit "$rc"
