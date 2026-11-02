#!/usr/bin/env bash
# engine-pin.sh — the repo certifies the engine it actually runs, by hash.
#
# WHY. On 2026-11-02 the night machinery moved into a skill so other projects could use it, and the project installed
# it with symlinks. The review that read the result named the cost in one line: "the gates certify copies of things
# that are not in the tree." A gate can be green while the engine that runs is whatever happens to be installed in
# ~/.hermes — a hash, not a path, is what makes that auditable, and this workspace already has the pattern
# (skills: deploy-drift, service-provenance).
#
# scripts/night/ENGINE.sha256 is the pin: one line per file, the hash of the ENGINES's bytes. This script recomputes
# and compares. Usage:
#   engine-pin.sh            compare (the gate's use); exit 1 on any difference
#   engine-pin.sh --write    re-pin after a deliberate upgrade, and the diff shows what changed
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1
ENGINE=${NL_ENGINE_DIR:-}
if [ -z "$ENGINE" ]; then
  first=$(readlink -f scripts/night/night.sh 2>/dev/null)
  ENGINE=$(dirname "${first:-/nonexistent}")   # the engine is wherever the first symlink really points
fi
PIN=${NL_ENGINE_PIN:-scripts/night/ENGINE.sha256}
FILES="lib.sh night.sh watch.sh doctor.sh land-row.sh fix-row.sh clear-stuck-tree.sh morning-report.sh state.py row-paths.py row-verify.py plan_import.py"

[ -d "$ENGINE" ] || { echo "REFUSED: no engine at '$ENGINE' — the symlinks point nowhere, so nothing can be certified"; exit 2; }

compute(){
  for f in $FILES; do
    [ -f "$ENGINE/$f" ] || { echo "MISSING $f"; continue; }
    printf '%s  %s\n' "$(sha256sum "$ENGINE/$f" | cut -d' ' -f1)" "$f"
  done
}

if [ "${1:-}" = "--write" ]; then
  compute >"$PIN" || exit 1
  echo "pinned $(wc -l <"$PIN" | tr -d ' ') engine file(s) from $ENGINE into $PIN"
  exit 0
fi

[ -f "$PIN" ] || { echo "REFUSED: $PIN is absent — the repo cannot say which engine it runs. Write it: bash $0 --write"; exit 2; }
now=$(compute | sort -k2)
pinned=$(sort -k2 "$PIN")
if [ "$now" = "$pinned" ]; then
  echo "ENGINE PIN OK: the installed engine matches $PIN ($(printf '%s' "$pinned" | grep -c . ) file(s), $ENGINE)"
  exit 0
fi
echo "ENGINE DRIFT: what runs is not what the repo pinned"
diff <(printf '%s\n' "$pinned") <(printf '%s\n' "$now") | sed 's/^/  /' | head -20
echo "  This is not automatically wrong — an upgrade is a decision. Re-pin with: bash $0 --write"
exit 1
