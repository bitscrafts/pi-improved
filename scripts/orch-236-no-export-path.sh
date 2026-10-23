#!/usr/bin/env bash
# Spec 236 R3: no SHARED test-step cache may carry a test binary that baked a manifest dir inside a
# fresh-gate export. The defect is precise: a Rust test binary bakes `env!("CARGO_MANIFEST_DIR")` at compile
# time, so a binary compiled inside an export names `<export>/crates/<crate>` (or `<export>/xtask`); the next
# run reuses it and reads a directory that the successful run deleted.
#
# The pattern therefore matches a baked manifest dir, not the bare prefix. Measured 2026-10-23: the bare
# prefix also matches the SAMPLE PATH that `fresh_gate::tests::gate_env_gives_the_test_step_a_target_inside_the_export`
# carries as test data (`/projects/_tmp/fresh-gate-7`), which any correct tree then reads as a violation;
# the entry also carries `/projects/pi-improved/_tmp/gate-target` (the warm dir) in the same string table.
# Both controls below are run every time: the pattern must see a baked manifest dir, and must not flag the
# sample path.
#
# Scope is deliberate and measured too. R2 keeps fmt/clippy/build/test-doc on the one warm cache
# (FRESH_GATE_TARGET), and those steps legitimately compile inside the export, so their artefacts carry the
# export path: on 2026-10-23 `grep -rl` over _tmp/gate-target found 13437 such files, of which the test step
# was a minority. The failing class is a stale TEST BINARY, so this check owns `*/test-gate` and reports how
# many hits sit outside its scope instead of pretending the wider property is achievable without the cold
# build R2 rules out.
# Exit 0 when every shared test-step cache is clean; exit 1 and list the hits otherwise.
set -u
cd "$(dirname "$0")/.." || exit 2
shopt -s nullglob

pattern="/projects/_tmp/fresh-gate-[0-9]+/(crates|xtask)/"

# Control 1: the checker must see a baked manifest dir when one is there.
# Control 2: it must NOT flag the bare sample path the unit test carries, because a check that fires on a
#            correct tree is a check everyone learns to ignore.
pos="_tmp/orch-236-control-pos-$$.txt"
neg="_tmp/orch-236-control-neg-$$.txt"
printf '%s\n' "/projects/_tmp/fresh-gate-999/crates/pii-evolve/src/driver.rs" >"$pos"
printf '%s\n' "/projects/_tmp/fresh-gate-7" >"$neg"
if ! grep -qE -- "$pattern" "$pos"; then
    echo "no-export-path: FAIL, control — the pattern cannot see a baked manifest dir"
    rm -f "$pos" "$neg"
    exit 1
fi
if grep -qE -- "$pattern" "$neg"; then
    echo "no-export-path: FAIL, control — the pattern flags the unit test's sample path"
    rm -f "$pos" "$neg"
    exit 1
fi
rm -f "$pos" "$neg"

scanned=0
total_hits=0
for d in _tmp/gate-target*/test-gate; do
    [ -d "$d" ] || continue
    scanned=$((scanned + 1))
    hits=$(grep -rlE -a -- "$pattern" "$d" 2>/dev/null)
    if [ -n "$hits" ]; then
        printf '%s\n' "$hits"
        n=$(printf '%s\n' "$hits" | wc -l)
        total_hits=$((total_hits + n))
        echo "no-export-path: $d carries $n export-baked file(s)" >&2
    fi
done

outside=0
for d in _tmp/gate-target*; do
    [ -d "$d" ] || continue
    n=$(grep -rlE -a -- "$pattern" "$d" 2>/dev/null | grep -vc "/test-gate/" || true)
    outside=$((outside + n))
done

if [ "$total_hits" -gt 0 ]; then
    echo "no-export-path: FAIL, $total_hits test-step file(s) baked a manifest dir inside an export"
    exit 1
fi
echo "no-export-path: PASS ($scanned shared test-step cache(s) scanned; controls ok; $outside export-baked file(s) outside the test step, which R2 keeps on the warm cache)"
