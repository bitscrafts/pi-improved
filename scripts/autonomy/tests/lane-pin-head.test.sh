#!/usr/bin/env bash
# Test: the spec lane requires its graphs (spec.toml, docs.toml, preflight.toml) to equal their content at HEAD, before the pin comparison (spec 302).
# Every refusal here is a dry run, so nothing is started and nothing is spent. The edited cases work in a scratch clone under _tmp/ whose copy of the
# lane script names the clone as its repository (the production script hard-codes the live one). The coder lane (PII_LANE_GRAPH unset) ignores the graphs.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
S=$REPO/scripts/run-batchC-pii.sh
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-pin-head-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; mv "$SCRATCH" "$REPO/_tmp/_trash/lane-pin-head-$(date +%s)"; }

# mkclone <name>: a scratch clone of HEAD, with the working-tree lane script copied in and pointed at the clone; prints its path
mkclone() {
  local c=$SCRATCH/$1
  git clone -q "$REPO" "$c" && sed 's|^REPO=/projects/pi-improved$|REPO='"$c"'|' "$S" > "$c/scripts/run-batchC-pii.sh" && echo "$c"
}
# pinned_edit <clone>: docs.toml gains a comment and the pin in spec.toml is repaired to match (the worker's edit)
pinned_edit() {
  printf '# worker edit\n' >> "$1/graphs/docs.toml"
  local sum; sum=$(sha256sum "$1/graphs/docs.toml" | cut -c1-64)
  sed -i "s/^sha256 = \"[0-9a-f]*\"/sha256 = \"$sum\"/" "$1/graphs/spec.toml"
}
# run_dry <clone> <lane: spec|coder>: a dry run of spec 302; stdout+stderr in <clone>.out, rc in <clone>.rc
run_dry() {
  local c=$1
  if [ "$2" = spec ]; then PII_LANE_GRAPH=spec bash "$c/scripts/run-batchC-pii.sh" 302 --dry-run > "$c.out" 2>&1
  else env -u PII_LANE_GRAPH bash "$c/scripts/run-batchC-pii.sh" 302 --dry-run > "$c.out" 2>&1; fi
  echo $? > "$c.rc"
}

# 1: the real repository's graphs equal HEAD, so the spec lane's dry run passes
PII_LANE_GRAPH=spec bash "$S" 302 --dry-run > "$SCRATCH/clean.out" 2>&1; CLEAN_RC=$?
if [ "$CLEAN_RC" = 0 ] && grep -q 'DRY RUN' "$SCRATCH/clean.out" && ! grep -q 'ABORT' "$SCRATCH/clean.out"; then ok "clean_graphs_pass"; else bad "clean_graphs_pass (rc [$CLEAN_RC])"; fi

# CONTROL 2: a matching edit of docs.toml and its pin is refused: the pin agrees, HEAD does not
C=$(mkclone matching); pinned_edit "$C"; run_dry "$C" spec
if [ "$(cat "$C.rc")" = 2 ] && grep -q 'differs from HEAD' "$C.out" && ! grep -q 'DRY RUN' "$C.out"; then ok "CONTROL_a_matching_edit_of_docs_and_pin_is_refused"; else bad "CONTROL_a_matching_edit_of_docs_and_pin_is_refused (rc [$(cat "$C.rc")])"; fi

# CONTROL 3: a graph removed from the index (untracked) is refused, though its content still equals HEAD
C=$(mkclone untracked); git -C "$C" rm -q --cached graphs/preflight.toml; run_dry "$C" spec
if [ "$(cat "$C.rc")" = 2 ] && grep -q 'differs from HEAD' "$C.out"; then ok "CONTROL_an_untracked_graph_is_refused"; else bad "CONTROL_an_untracked_graph_is_refused (rc [$(cat "$C.rc")])"; fi

# 4: the coder lane ignores the graph edits: the same edited clone dry-runs without the HEAD line
C=$(mkclone coder); pinned_edit "$C"; run_dry "$C" coder
if ! grep -q 'differs from HEAD' "$C.out" && grep -q 'DRY RUN' "$C.out"; then ok "coder_lane_ignores_graph_edits"; else bad "coder_lane_ignores_graph_edits (rc [$(cat "$C.rc")])"; fi

# CONTROL 5: a docs.toml edit whose pin is NOT repaired is refused by HEAD, not only by the pin
C=$(mkclone docs-only); printf '# worker edit\n' >> "$C/graphs/docs.toml"; run_dry "$C" spec
if [ "$(cat "$C.rc")" = 2 ] && grep -q 'differs from HEAD' "$C.out"; then ok "CONTROL_a_docs_only_edit_is_refused_by_head"; else bad "CONTROL_a_docs_only_edit_is_refused_by_head (rc [$(cat "$C.rc")])"; fi

# CONTROL 6: a preflight.toml edit is refused too: the third graph is checked, not only the pinned pair
C=$(mkclone preflight-only); printf '# worker edit\n' >> "$C/graphs/preflight.toml"; run_dry "$C" spec
if [ "$(cat "$C.rc")" = 2 ] && grep -q 'differs from HEAD' "$C.out"; then ok "CONTROL_a_preflight_edit_is_refused_by_head"; else bad "CONTROL_a_preflight_edit_is_refused_by_head (rc [$(cat "$C.rc")])"; fi

# CONTROL 7 (review of A166): a spec.toml edit alone is refused by HEAD. Every earlier case edits docs.toml or preflight.toml, or repairs the pin through spec.toml,
# so a check that skipped spec.toml survived mutation. The pin still matches docs.toml here, so only the HEAD comparison can refuse it.
C=$(mkclone spec-only); printf '# worker edit\n' >> "$C/graphs/spec.toml"; run_dry "$C" spec
if [ "$(cat "$C.rc")" = 2 ] && grep -q 'differs from HEAD' "$C.out" && grep -q 'graphs/spec.toml' "$C.out"; then ok "CONTROL_a_spec_only_edit_is_refused_by_head"; else bad "CONTROL_a_spec_only_edit_is_refused_by_head (rc [$(cat "$C.rc")])"; fi

trash
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
