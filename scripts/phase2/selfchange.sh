#!/usr/bin/env bash
# E13: Pii makes a real multi-file change in its own repo driven by
# starter/react (F6.1). The scripted selfchange fixture issues two `edit`
# calls against two distinct files, then a no-tool-call turn. Writes only
# under _tmp/phase2/. A stale scratch dir is moved aside, never removed.
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

FIXTURE="$ROOT/fixtures/phase2/selfchange.json"
GRAPH="$ROOT/graphs/starter/react.toml"
PII_BIN="$ROOT/target/debug/pii"
T="$ROOT/_tmp/phase2/selfrepo"
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi
mkdir -p "$T"
cd "$T"
git init -q
git config user.email test@example.com
git config user.name test
printf 'line old-a\n' > a.txt
printf 'line old-b\n' > b.txt
git add a.txt b.txt
git commit -qm init

# Run pii -p with the scripted selfchange fixture and the react graph.
# The loopback engine resolves --graph <name> from <workspace>/.pii/graphs/.
mkdir -p "$T/.pii/graphs"
cp "$GRAPH" "$T/.pii/graphs/react.toml"
PII_RUNTIME_DIR="$ROOT/_tmp/phase2/rt" PII_MODEL="scripted:$FIXTURE" \
    "$PII_BIN" -p "fix both files" --json --graph "react" \
    > "$ROOT/_tmp/phase2/selfchange.ndjson" 2>&1

# The run must have finished successfully.
grep -q '"kind":"run_finished"' "$ROOT/_tmp/phase2/selfchange.ndjson" \
    || { echo "FAIL: no run_finished in ndjson" >&2; exit 1; }

# Both files must have been edited (git status shows >= 2 changes).
CHANGED=$(git status --porcelain | wc -l)
if [ "$CHANGED" -lt 2 ]; then
    echo "FAIL: expected >= 2 changed files, got $CHANGED" >&2
    git status --porcelain >&2
    exit 1
fi
grep -q 'new-a' a.txt || { echo "FAIL: a.txt not edited" >&2; exit 1; }
grep -q 'new-b' b.txt || { echo "FAIL: b.txt not edited" >&2; exit 1; }

echo "selfchange: OK ($CHANGED files changed)"
