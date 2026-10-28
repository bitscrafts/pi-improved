#!/usr/bin/env bash
# Test: the composed lane supplies every $.input.<key> the spec graph and the docs graph read (spec 296). The lane's input comes from a dry
# run of scripts/run-batchC-pii.sh with PII_LANE_GRAPH=spec (it writes the input and exits BEFORE the daemon, ssh or any agent). A graph key
# the lane does not supply is a FAIL (the run would be refused at start); a lane key no graph reads is a NOTE. Controls: the real input
# without docs_brief, and a copy of spec.toml with a new key.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
# uv (the input builder) keeps its cache and managed Pythons under the repo's _tmp when the environment gives it no writable home
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
S=$REPO/scripts/run-batchC-pii.sh
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-input-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; for f in "$@"; do [ -e "$f" ] && mv "$f" "$REPO/_tmp/_trash/lane-input-$(date +%s)-$(basename "$f")"; done; }
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line
graph_keys() { # <graph.toml>... -> the distinct $.input.<key> names, one per line, sorted
  grep -ohE '\$\.input\.[a-z_]+' "$@" | sed 's/^\$\.input\.//' | sort -u
}
lane_keys() { # <input.json> -> the keys of the input the lane wrote, one per line, sorted
  uv run --no-project python -I -c 'import json,sys; print("\n".join(sorted(json.load(open(sys.argv[1])))))' "$1"
}
compare() { # <graph-keys> <lane-keys> -> FAIL lines for graph keys the lane lacks, a NOTE line per lane-only key; 0 when none is missing
  local miss k
  miss=$(comm -23 "$1" "$2")
  for k in $(comm -13 "$1" "$2"); do echo "NOTE lane supplies $k, no graph reads it"; done
  for k in $miss; do echo "FAIL missing from the lane: $k"; done
  [ -z "$miss" ]
}

# the real lane input. PII_GRAPH_EDIT_OK (spec 303 R5): this test only dry-runs, and it must still work inside a row that edits a graph;
# the opt-out is a dry-run-only door — a real start compares the graphs with HEAD whatever it says.
OUT=$(PII_LANE_GRAPH=spec PII_GRAPH_EDIT_OK=1 timeout 120 bash "$S" 296 --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" != 0 ] || [ ! -f "$I" ]; then echo "FAIL the dry run wrote no input (exit $RC): $(printf '%s' "$OUT" | tail -2 | tr '\n' ' ')"; echo "PASS: 0 FAIL: 1"; exit 1; fi
GK=$SCRATCH/graph-keys.txt; LK=$SCRATCH/lane-keys.txt
graph_keys "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" > "$GK"
lane_keys "$I" > "$LK"

R=$(compare "$GK" "$LK"); RC=$?
if [ -n "$(graph_keys "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml")" ] && [ "$RC" = 0 ]; then ok "lane_supplies_every_key_the_graphs_read: $(wc -l < "$GK") graph keys all supplied"
else bad "lane_supplies_every_key_the_graphs_read: $(printf '%s' "$R" | grep '^FAIL' | tr '\n' ' ')"; fi
printf '%s\n' "$R" | grep '^NOTE' || true

# CONTROL: the real input without docs_brief must be caught as missing
CUT=$SCRATCH/input-without-docs_brief.json
uv run --no-project python -I -c 'import json,sys; d=json.load(open(sys.argv[1])); d.pop("docs_brief"); open(sys.argv[2],"w").write(json.dumps(d))' "$I" "$CUT"
lane_keys "$CUT" > "$SCRATCH/cut-keys.txt"
R=$(compare "$GK" "$SCRATCH/cut-keys.txt"); RC=$?
if [ "$RC" != 0 ] && printf '%s' "$R" | grep -q '^FAIL missing from the lane: docs_brief$'; then ok "CONTROL_a_missing_docs_brief_is_caught"
else bad "CONTROL_a_missing_docs_brief_is_caught: exit $RC, output [$R]"; fi

# CONTROL: a copy of spec.toml with a new key the lane does not supply must be caught
mkdir -p "$SCRATCH/graphs"
cp "$REPO/graphs/spec.toml" "$SCRATCH/graphs/spec.toml"
printf '\nzz_new_key = "$.input.zz_new_key"\n' >> "$SCRATCH/graphs/spec.toml"
graph_keys "$SCRATCH/graphs/spec.toml" "$REPO/graphs/docs.toml" > "$SCRATCH/graph-keys-new.txt"
R=$(compare "$SCRATCH/graph-keys-new.txt" "$LK"); RC=$?
if [ "$RC" != 0 ] && printf '%s' "$R" | grep -q '^FAIL missing from the lane: zz_new_key$'; then ok "CONTROL_a_new_graph_key_is_caught"
else bad "CONTROL_a_new_graph_key_is_caught: exit $RC, output [$R]"; fi

# a lane key no graph reads is a NOTE, not a failure
EXTRA=$SCRATCH/input-with-extra.json
uv run --no-project python -I -c 'import json,sys; d=json.load(open(sys.argv[1])); d["zz_lane_only"]="x"; open(sys.argv[2],"w").write(json.dumps(d))' "$I" "$EXTRA"
lane_keys "$EXTRA" > "$SCRATCH/extra-keys.txt"
R=$(compare "$GK" "$SCRATCH/extra-keys.txt"); RC=$?
if [ "$RC" = 0 ] && printf '%s' "$R" | grep -q '^NOTE lane supplies zz_lane_only, no graph reads it$' && ! printf '%s' "$R" | grep -q '^FAIL'; then ok "lane_extra_keys_are_a_note_not_a_failure"
else bad "lane_extra_keys_are_a_note_not_a_failure: exit $RC, output [$R]"; fi

trash "$I" "$(field "$OUT" GRAPH)"
trash "$SCRATCH"
echo "PASS: $PASS FAIL: $FAIL"
