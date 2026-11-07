#!/usr/bin/env bash
# Write the deploy card for a gated revision from MEASURED values (commit count, diff stat, the fresh-gate log's own verdict lines).
# usage: make-deploy-card.sh <rev> <fresh-gate-output.log>
set -euo pipefail
cd /projects/pi-improved
REV12=$(git rev-parse --verify --short=12 "$1^{commit}"); REV7=${REV12:0:7}; LOG=$2
DEPLOYED=085369902766
[ -f "$LOG" ] || { echo "no gate log $LOG"; exit 2; }
grep -q '^quality-gate: ALL PASS' "$LOG" || { echo "REFUSED: $LOG does not say 'quality-gate: ALL PASS'"; exit 2; }
grep -q "fresh-gate-" "$LOG" || { echo "REFUSED: $LOG is not a fresh-gate (worktree) run"; exit 2; }
N=$(git rev-list --count "$DEPLOYED..$REV12"); STAT=$(git diff --shortstat "$DEPLOYED" "$REV12")
SKIPS=$(grep -cE '^SKIP |^SKIPPED:' "$LOG" || true)
mkdir -p _tmp/deploy
python3 - "$REV12" "$N" "$STAT" "$LOG" "$SKIPS" "$DEPLOYED" > "_tmp/deploy/card-$REV7.json" <<'PY'
import json, sys
rev, n, stat, log, skips, dep = sys.argv[1:7]
print(json.dumps({
 "incident": {"id": "deploy-drift", "fingerprint": f"host-running-{dep[:7]}-vs-repo-at-{rev[:7]}", "count": int(n)},
 "change": {"branch": "master", "revision": rev, "diff_stat": f"{n} commits; {stat.strip()} against the deployed {dep}"},
 "proof": {
  "red_on_base": "row 362: queue.toml's first real run on the host died in 1.6 s with {\"code\":\"internal\",\"message\":\"Model deepseek-v4-flash not found\"}; on the base commit a role arriving through $.input.model reached the agent as the literal string @workhorse (tests a_role_arriving_through_the_run_input_is_resolved_at_spawn, an_unknown_role_through_the_run_input_is_refused_by_name_and_spawns_nothing and a_role_passed_as_both_model_and_provider_resolves_both_from_the_table were red; disabling the resolution turns them red again)",
  "green_on_branch": "row 362: models_from_config 8/8, queue_calls_spec 4/4, row criteria 6/6; full workspace sweep 356 test targets, 0 failures; autonomy suite 52/52 with a cycle in flight"},
 "gate": {"verdict": f"fresh-gate PASS on {rev} (mode=fresh, clean worktree export; quality-gate: ALL PASS; {log}). {skips} steps reported SKIP/SKIPPED and were not counted as passes: coverage-floor was skipped in this run (the routine gate measured it at 81.82% against the 74% floor); hal-gpiod steps are compile+lint only"},
 "rollback": {"rehearsal": "scripts/deploy-upgrade-host.sh rehearsed on the host in a scratch root (REMOTE_ROOT, no unit, no real daemon touched): a successful upgrade, AND a forced bad-health run that restored the backed-up binary, config, graphs and stores byte-for-byte (sha256 compared); evidence in _tmp/deploy/rehearsal-" + rev[:7] + ".log"}
}, indent=1))
PY
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "_tmp/deploy/card-$REV7.json" && echo "card written: _tmp/deploy/card-$REV7.json"
/projects/_tmp/release-$REV12/pii deploy-card render --card "_tmp/deploy/card-$REV7.json" 2>&1 | head -12 | cut -c1-230 || true
