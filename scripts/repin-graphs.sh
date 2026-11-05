#!/usr/bin/env bash
# repin-graphs.sh — bring every `call` pin in `graphs/` up to the file it names.
#
# Why this is a script and not a sed (2026-11-04): editing one graph invalidates the pin of every graph
# that calls it, which invalidates the pin of every graph that calls THOSE. Measured tonight: one edit
# to `sdd.toml` left `spec`, `night`, `queue` and `self-improve` unable to load, each naming a
# different call. A pin is a promise that the reviewed bytes are the executed bytes; keeping it honest
# has to be one command that reaches a fixed point, not five hand edits done in whatever order.
#
# This is the manual form of `pii graph repin <graph>` from derivation 047. When that verb exists this
# script should become its test.
#
# Usage: repin-graphs.sh [--dry-run]
set -uo pipefail
cd /projects/pi-improved || exit 1
DRY=${1:-}
changed_total=0
for round in 1 2 3 4 5; do
  changed_this=0
  for f in graphs/*.toml; do
    [ -f "$f" ] || continue
    python3 - "$f" "$DRY" <<'PY'
import hashlib, pathlib, re, sys
path = pathlib.Path(sys.argv[1]); dry = sys.argv[2] == "--dry-run"
text = path.read_text(); orig = text
# every call site is `graph = "<name>"` followed by `sha256 = "<digest>"`
def fix(m):
    name, pin = m.group(1), m.group(2)
    target = pathlib.Path("graphs") / f"{name}.toml"
    if not target.exists():
        return m.group(0)                     # a user graph, or a missing one: leave it to the lint
    want = hashlib.sha256(target.read_bytes()).hexdigest()
    return m.group(1) if False else f'graph = "{name}"\nsha256 = "{want}"' if want != pin else m.group(0)
text = re.sub(r'graph = "([a-z0-9_-]+)"\nsha256 = "([0-9a-f]{64})"', fix, text)
if text != orig and not dry:
    path.write_text(text)
if text != orig:
    print(f"repin {path.name}")
    sys.exit(1)
sys.exit(0)
PY
    [ $? -eq 1 ] && changed_this=$((changed_this+1))
  done
  changed_total=$((changed_total+changed_this))
  [ "$changed_this" -eq 0 ] && break
done
echo "repin-graphs: $changed_total file(s) changed over $round round(s)"
[ -n "$DRY" ] && echo "(dry run: nothing written)"
exit 0
