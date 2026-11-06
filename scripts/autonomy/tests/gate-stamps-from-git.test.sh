#!/usr/bin/env bash
# gate-stamps-from-git.test.sh - the gate's build must not inherit PII_BUILD_REVISION from whoever launched the cycle.
#
# MEASURED 2026-11-06: a stale PII_BUILD_REVISION in the launching shell reached the gate, build.rs preferred it to git, every
# binary was stamped bc51317 while HEAD was 49549a5, and lane_run_parity refused it. This runs the gate line's own environment
# handling: it extracts the gate subshell from the real cycle.sh and executes it with `env` as the gated command.
cd "$(dirname "$0")/../../.." || exit 1
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "ok   $1"; }
bad() { FAIL=$((FAIL+1)); echo "FAIL $1"; }
LINE=$(grep -n 'bash quality-gate.sh --routine' scripts/autonomy/cycle.sh | head -1 | cut -d: -f1)
[ -n "$LINE" ] || { echo "FAIL cannot find the gate line in cycle.sh"; exit 1; }
SUB=$(sed -n "${LINE}p" scripts/autonomy/cycle.sh | sed -e 's/^ *//' -e 's/ >>"\$LOG".*$//' -e 's/bash quality-gate.sh --routine/env/' -e 's/timeout "\$GATE_TIMEOUT" //')
gate_skip_coverage() { echo 0; }
GOT=$(PII_BUILD_REVISION=deadbeef0000 bash -c "gate_skip_coverage() { echo 0; }; GATE_TIMEOUT=60; $SUB" 2>&1 | grep -c '^PII_BUILD_REVISION=')
[ "$GOT" = 0 ] && ok "a stale PII_BUILD_REVISION does not reach the gated command" || bad "PII_BUILD_REVISION reached the gated command ($GOT)"
# CONTROL: the same extraction still passes the variables the gate IS meant to set
KEPT=$(PII_BUILD_REVISION=deadbeef0000 bash -c "gate_skip_coverage() { echo 0; }; GATE_TIMEOUT=60; $SUB" 2>&1 | grep -c '^GATE_STEPS=gate-steps.txt')
[ "$KEPT" = 1 ] && ok "CONTROL: the gate's own variables are still set" || bad "CONTROL: GATE_STEPS lost ($KEPT)"
# CONTROL 2: without the unset the variable WOULD reach it (proves the extraction can see the variable at all)
NO=$(PII_BUILD_REVISION=deadbeef0000 bash -c "gate_skip_coverage() { echo 0; }; GATE_TIMEOUT=60; ${SUB/unset PII_BUILD_REVISION;/:}" 2>&1 | grep -c '^PII_BUILD_REVISION=deadbeef0000')
[ "$NO" = 1 ] && ok "CONTROL: with the unset removed the stale value is visible" || bad "CONTROL: extraction cannot see the variable ($NO)"
echo "PASS: $PASS FAIL: $FAIL"; [ "$FAIL" = 0 ]
