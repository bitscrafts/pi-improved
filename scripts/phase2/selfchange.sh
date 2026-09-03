#!/usr/bin/env bash
# E13 (F5, opus review): Pii makes a real multi-file change in a copy of its
# own repo driven by starter/react, then runs `cargo test --workspace` inside
# the copy and propagates its exit code. Writes only under _tmp/phase2/. A
# stale scratch dir is moved aside, never removed.
set -euo pipefail
export PATH=/usr/local/cargo/bin:$PATH
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

FIXTURE="$ROOT/fixtures/phase2/selfchange.json"
GRAPH="react"
PII_BIN="$ROOT/target/debug/pii"
T="$ROOT/_tmp/phase2/selfrepo"
if [ -d "$T" ]; then
    mv "$T" "$T.old-$(date +%s)"
fi

# Copy Pii's own repo (excluding target/, _tmp/, .git) into the scratch dir.
mkdir -p "$T"
cp -r "$ROOT"/{crates,graphs,fixtures,specs,xtask,examples,Cargo.toml,Cargo.lock,clippy.toml,rustfmt.toml,rust-toolchain.toml} "$T/" 2>/dev/null || true
# Seed the two files the scripted fixture edits (they are the change target).
printf 'line old-a\n' > "$T/a.txt"
printf 'line old-b\n' > "$T/b.txt"
cd "$T"
git init -q
git config user.email test@example.com
git config user.name test
git add -A
git commit -qm init

# Run pii -p with the scripted selfchange fixture and the react graph.
# The loopback engine resolves --graph <name> from <workspace>/.pii/graphs/.
mkdir -p "$T/.pii/graphs"
cp "$ROOT/graphs/starter/react.toml" "$T/.pii/graphs/react.toml"
PII_RUNTIME_DIR="$ROOT/_tmp/phase2/rt" PII_MODEL="scripted:$FIXTURE" \
    "$PII_BIN" -p "fix both files" --json --graph "$GRAPH" \
    > "$ROOT/_tmp/phase2/selfchange.ndjson" 2>&1

# The run must have finished successfully.
grep -q '"kind":"run_finished"' "$ROOT/_tmp/phase2/selfchange.ndjson" \
    || { echo "FAIL: no run_finished in ndjson" >&2; exit 1; }
if grep -q '"kind":"failure"' "$ROOT/_tmp/phase2/selfchange.ndjson"; then
    echo "FAIL: run_finished with failure terminal" >&2
    exit 1
fi

# Both files must have been edited (git status shows >= 2 changes).
CHANGED=$(git status --porcelain | wc -l)
if [ "$CHANGED" -lt 2 ]; then
    echo "FAIL: expected >= 2 changed files, got $CHANGED" >&2
    git status --porcelain >&2
    exit 1
fi
grep -q 'new-a' a.txt || { echo "FAIL: a.txt not edited" >&2; exit 1; }
grep -q 'new-b' b.txt || { echo "FAIL: b.txt not edited" >&2; exit 1; }

# F5: the change must not break the workspace — run cargo test in the copy.
if ! cargo test --workspace > "$ROOT/_tmp/phase2/selfchange-test.log" 2>&1; then
    echo "FAIL: cargo test --workspace failed in the copied repo" >&2
    tail -20 "$ROOT/_tmp/phase2/selfchange-test.log" >&2
    exit 1
fi

echo "selfchange: OK ($CHANGED files changed, cargo test passed)"
