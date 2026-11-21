#!/usr/bin/env bash
# Spec 430: the two graphs that make the worker's container are linted together, and a graph the linter refuses fails
# this check. `pii graph lint` is the only reader of the graph schema; it returns 0 for a dangling port (V5), so this
# script's job is the one verdict that IS fatal — a finding it reports as an error — and it prints every finding so a
# person can still read the warnings.
#
# Usage (from the repository root): bash scripts/checks/worker-runtime-graph.sh
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)" || exit 2
cd "$root" || exit 2

pii="${PII:-pii}"
if ! command -v "$pii" >/dev/null 2>&1; then
  echo 'FAIL: no `pii` on PATH; set PII=/path/to/pii' >&2
  exit 2
fi

failed=0
for graph in graphs/podman.toml graphs/worker-runtime.toml; do
  if [ ! -f "$graph" ]; then
    echo "FAIL $graph: missing" >&2
    failed=1
    continue
  fi
  out="$("$pii" graph lint "$graph" 2>&1)"
  rc=$?
  printf '%s\n' "$out"
  echo "LINT $graph rc=$rc"
  if [ "$rc" -ne 0 ]; then
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "FAIL worker-runtime-graph: a graph does not lint" >&2
  exit 1
fi
echo "OK worker-runtime-graph: 2 graphs lint"