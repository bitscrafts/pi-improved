#!/usr/bin/env bash
# Test: the graph run-batchC-pii.sh GENERATES carries a wall limit that outlives its coder node. The input is the live artefact: the REAL
# script is run (DEVBOX_HOST unreachable, so it writes its graph and then fails at the first ssh), and the graph file it left is parsed.
# A control runs a copy of the script with the [limits] block removed: the same check must FAIL on it.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
REPO=$(pwd); PASS=0; FAIL=0
ok()   { echo "  ok   $1"; PASS=$((PASS+1)); }
bad()  { echo "  FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-wall-XXXXXX") || exit 2
render() { # <script> <timeout-ms|-> -> echoes the generated graph path, "" when none
  local S=$1 T=$2 BEFORE AFTER
  BEFORE=$(ls "$REPO"/_tmp/devbox-test/graphs/q286-*.toml 2>/dev/null | sort)
  if [ "$T" = "-" ]; then
    ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1 timeout 60 bash "$S" 286 >/dev/null 2>&1
  else
    PII_NODE_TIMEOUT_MS=$T ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1 timeout 60 bash "$S" 286 >/dev/null 2>&1
  fi
  AFTER=$(ls "$REPO"/_tmp/devbox-test/graphs/q286-*.toml 2>/dev/null | sort)
  comm -13 <(echo "$BEFORE") <(echo "$AFTER") | tail -1
}
relation() { # <graph> -> prints "wall timeout" ; empty wall when absent
  uv run --no-project python - "$1" <<'PY'
import sys, tomllib
d = tomllib.load(open(sys.argv[1], "rb"))
wall = d.get("limits", {}).get("max_wall_ms", "none")
node = next(n for n in d["node"] if n["id"] == "code")
print(wall, node["timeout_ms"])
PY
}
trash() { mkdir -p "$REPO/_tmp/_trash"; for f in "$@"; do [ -e "$f" ] && mv "$f" "$REPO/_tmp/_trash/lane-wall-$(date +%s)-$(basename "$f")"; done; }
check() { # <label> <script> <timeout|-> <expect pass|fail>
  local G; G=$(render "$2" "$3")
  [ -n "$G" ] || { bad "$1: the script left no graph"; return; }
  read -r WALL TO <<<"$(relation "$G")"
  # Both fields must be present before anything is compared: a shifted or empty field must be a loud error, never a verdict.
  case "$TO" in ''|*[!0-9]*) bad "$1: unreadable node timeout '$TO' (wall '$WALL')"; trash "$G"; return;; esac
  local HOLDS=no
  case "$WALL" in ''|*[!0-9]*) ;; *) [ "$WALL" -ge "$TO" ] && HOLDS=yes;; esac
  case "$4:$HOLDS" in
    pass:yes) ok "$1: wall $WALL >= node timeout $TO";;
    fail:no)  ok "$1: control fails as it must (wall='$WALL', node timeout $TO)";;
    pass:no)  bad "$1: wall '$WALL' does not cover the node timeout $TO";;
    fail:yes) bad "$1: the control did not fail - the check cannot tell";;
  esac
  trash "$G"
}
echo "== the real script, default node timeout"; check default "$REPO/scripts/run-batchC-pii.sh" - pass
echo "== the real script, a LONGER node timeout (the relation must follow it)"; check longer "$REPO/scripts/run-batchC-pii.sh" 5000000 pass
echo "== control: the same script with the [limits] block removed"
uv run --no-project python - "$REPO/scripts/run-batchC-pii.sh" "$SCRATCH/no-limits.sh" <<'PY'
import sys, re
s = open(sys.argv[1]).read()
t = s.replace("[limits]\nmax_wall_ms = $WALL_MS\n\n", "")
assert t != s, "no [limits] block to remove: the control has nothing to remove"
open(sys.argv[2], "w").write(t)
PY
sed -i "s|^REPO=/projects/pi-improved|REPO=$REPO|" "$SCRATCH/no-limits.sh"
check no-limits "$SCRATCH/no-limits.sh" - fail
trash "$SCRATCH"
echo "PASS: $PASS FAIL: $FAIL"