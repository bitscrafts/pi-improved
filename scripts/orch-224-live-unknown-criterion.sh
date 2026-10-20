#!/usr/bin/env bash
# Spec 224 criterion 4, as a command: on lnx, a coder node whose criterion is not in the named table
# (`criteria = ["cargo test"]`) produces a run whose error string contains `criteria.unknown`, and the
# run-level `coder.no_call_events` never replaces it. Driven through the built binary's own
# `pii graph run`, so the criteria table is the production one — not the test fixture's.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.." || exit 1

BIN=${BIN:-./target/debug/pii}
[ -x "$BIN" ] || { echo "FAIL: no binary at $BIN"; exit 2; }

SCRATCH=$(mktemp -d _tmp/orch-224-live-XXXXXX) || exit 2
trap 'rmdir "$SCRATCH" 2>/dev/null' EXIT
mkdir -p "$SCRATCH/work"

cat >"$SCRATCH/graph.toml" <<TOML
name = "orch-224-live-unknown-criterion"
entry = "code"
caps = ["spawn", "proc"]

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
agent = "pi"
model = "m"
brief = "b"
spec = "specs/224-a-refused-criterion-fails-the-node.md"
workdir = "$PWD/$SCRATCH/work"
criteria = ["cargo test"]

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "transform"
value = "refused"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
TOML

OUT=$("$BIN" graph run "$SCRATCH/graph.toml" --db "$SCRATCH/store.redb" --run-id "o224$$" 2>&1)
RC=$?
echo "$OUT" | grep -E '"NodeFailed"|"terminal"' | tail -2

fail=0
# the run's own terminal error carries the node's refusal, verbatim
printf '%s' "$OUT" | grep -q '"terminal":{"Failure":{"error":"criteria.unknown' \
  || { echo "FAIL: the run terminal does not carry criteria.unknown"; fail=1; }
# the node failed with the same reason, not a value on the err port
printf '%s' "$OUT" | grep -q '"NodeFailed":{"node":"code","error":"criteria.unknown' \
  || { echo "FAIL: the node did not fail with criteria.unknown"; fail=1; }
# the run-level check must never overwrite the node-level one
printf '%s' "$OUT" | grep -q 'coder.no_call_events' \
  && { echo "FAIL: the run reported coder.no_call_events again"; fail=1; }

if [ "$fail" -eq 0 ]; then echo "PASS: criteria.unknown reaches the run's terminal error"; fi
# graph run reports the failure as its exit status too; that is the expected outcome here, not an error
echo "graph-run rc=$RC (a failing run exits non-zero by design)"
exit "$fail"
