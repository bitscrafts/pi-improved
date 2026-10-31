#!/usr/bin/env bash
# requires: dev-tree
# why: it clones /projects/pi-improved by absolute path, so it only means anything in the dev tree
# Test: the spec lane preflights the devbox before it spends, and logs why a refused run was refused (spec 298). The real lane script runs with a
# stub `ssh` first on PATH: it records every remote command and answers from a fixture directory, so no daemon starts and nothing is spent.
# The coder lane (PII_LANE_GRAPH unset) must keep today's behaviour: no preflight.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
S=$REPO/scripts/run-batchC-pii.sh
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-preflight-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; mv "$SCRATCH" "$REPO/_tmp/_trash/lane-preflight-$(date +%s)"; }
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line

# the stub ssh: its last argument is the remote command; it answers from the fixture directory $STUB_FIX
mkdir -p "$SCRATCH/bin"
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/usr/bin/env bash
F=$STUB_FIX
echo "$*" >> "$F/argv"
cmd="${*: -1}"
case "$cmd" in
  *"mkdir -p /projects/_tmp"*) echo "preflight stub"; exit "$(cat "$F/preflight_rc" 2>/dev/null || echo 0)" ;;
  *"command -v uv"*) echo "preflight stub"; exit "$(cat "$F/preflight_rc" 2>/dev/null || echo 0)" ;;
  *"run trace"*) cat "$F/trace" 2>/dev/null ;;
  *"run explain"*) echo "explain: refused, see the trace" ;;
  *"run start preflight"*) echo "run r000000000418 started" ;;
  *"run get r000000000418"*) echo "{\"status\":\"$(cat "$F/preflight_status")\"}" ;;
  *"run get"*) echo "{\"status\":\"$(cat "$F/status")\"}" ;;
  *"start-"*) echo "run r000000000419 started" ;;
  *"coder.spawned"*) echo '{"kind":"coder.spawned","session":"sid-298"}' ;;
  *"coder.exited"*) echo '{"kind":"coder.exited","rc":"0","tree_changed":"yes","quota_exhausted":"no"}' ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/ssh"

# the stub daemon script (PII_DAEMON_SH): the real one uses its own ssh
cat > "$SCRATCH/daemon.sh" <<'DSTUB'
#!/usr/bin/env bash
echo '"revision":"stub000000000"'
DSTUB
chmod +x "$SCRATCH/daemon.sh"

# fx <name> <final status> [trace text] [preflight rc] [preflight judge status]: a fresh fixture directory
fx() { local d=$SCRATCH/$1; mkdir -p "$d"; : > "$d/argv"; printf '%s\n' "$2" > "$d/status"; printf '%s\n' "${3:-}" > "$d/trace"; printf '%s\n' "${4:-0}" > "$d/preflight_rc"; printf '%s\n' "${5:-succeeded}" > "$d/preflight_status"; echo "$d"; }
# lane <fixture dir> [args...]: the real lane script (spec lane unless LANE_GRAPH says otherwise); stdout+stderr in <dir>/out, rc in <dir>/rc
lane() {
  local d=$1; shift
  STUB_FIX=$d PII_DAEMON_SH="$SCRATCH/daemon.sh" PII_SSH="$SCRATCH/bin/ssh" PATH="$SCRATCH/bin:$PATH" PII_LANE_GRAPH=${LANE_GRAPH-spec} bash "$S" 298 "$@" > "$d/out" 2>&1
  echo $? > "$d/rc"
}
TRACE_CRIT='13 verify - crit=7/8 fail=bash scripts/checks/live-orphan-count.sh'

# 1 (spec 301, replaces the 298 test): the uv check and the preflight judge run are both recorded before the real run starts
D=$(fx start succeeded); lane "$D"
UV=$(grep -n 'command -v uv' "$D/argv" | head -1 | cut -d: -f1)
PRE=$(grep -n 'run start preflight' "$D/argv" | head -1 | cut -d: -f1)
RUN=$(grep -n 'start-' "$D/argv" | head -1 | cut -d: -f1)
if [ -n "$UV" ] && [ -n "$PRE" ] && [ -n "$RUN" ] && [ "$UV" -lt "$RUN" ] && [ "$PRE" -lt "$RUN" ]; then ok "preflight_runs_before_the_run_starts"; else bad "preflight_runs_before_the_run_starts (uv line [$UV], preflight line [$PRE], run start line [$RUN])"; fi

# 2 (spec 301, R4): the uv check still runs inside the container, through podman exec
if grep -q 'podman exec pii-devbox sh -c .command -v uv' "$D/argv"; then ok "preflight_uv_check_runs_in_the_container"; else bad "preflight_uv_check_runs_in_the_container"; fi

# CONTROL 3: a failing preflight aborts before anything is started
D=$(fx preflight-fails succeeded "" 1); lane "$D"
if [ "$(cat "$D/rc")" = 2 ] && grep -q 'ABORT: container preflight failed' "$D/out" && ! grep -q 'start-' "$D/argv"; then ok "CONTROL_a_failing_preflight_aborts_with_nothing_started"; else bad "CONTROL_a_failing_preflight_aborts_with_nothing_started (rc [$(cat "$D/rc")])"; fi

# 4: a dry run prints the skip and never calls ssh
D=$(fx dry succeeded); lane "$D" --dry-run
if grep -q 'PREFLIGHT: skipped (dry run)' "$D/out" && [ ! -s "$D/argv" ]; then ok "dry_run_skips_the_preflight"; else bad "dry_run_skips_the_preflight"; fi

# 5: the coder lane has no preflight
D=$(fx coder succeeded); LANE_GRAPH= lane "$D" --dry-run
if ! grep -q 'PREFLIGHT:' "$D/out"; then ok "coder_lane_has_no_preflight"; else bad "coder_lane_has_no_preflight"; fi

# 6: a refused run logs the failing criterion, taken from the trace
D=$(fx refused failed "$TRACE_CRIT"); lane "$D"
LOGF=$(field "$(cat "$D/out")" LOG)
if grep -q 'why the run was not accepted' "$D/out" && grep -qF "$TRACE_CRIT" "$D/out" && grep -qF "$TRACE_CRIT" "$LOGF"; then ok "refused_run_logs_the_failing_criterion"; else bad "refused_run_logs_the_failing_criterion (log [$LOGF])"; fi

# CONTROL 7: a succeeded run logs no reason, even when its trace names a criterion
D=$(fx succeeded succeeded "$TRACE_CRIT"); lane "$D"
if ! grep -q 'why the run was not accepted' "$D/out"; then ok "CONTROL_a_succeeded_run_logs_no_reason"; else bad "CONTROL_a_succeeded_run_logs_no_reason"; fi

# 8: a refused run whose trace names no criterion says so
D=$(fx nocrit failed "13 code - ok"); lane "$D"
if grep -qF '== why the run was not accepted: (the trace names no criterion)' "$D/out"; then ok "trace_without_a_criterion_says_so"; else bad "trace_without_a_criterion_says_so"; fi

# 9 (spec 301): the brief names the repository's scratch, never /projects/_tmp
D=$(fx brief succeeded); lane "$D" --dry-run
BRIEF=$(sed -n 's/^BRIEF    : \([^ ]*\) .*/\1/p' "$D/out" | head -1)
if [ -f "$BRIEF" ] && grep -q 'under _tmp/ in the repository' "$BRIEF" && ! grep -q '/projects/_tmp' "$BRIEF"; then ok "brief_names_the_repo_scratch"; else bad "brief_names_the_repo_scratch (brief [$BRIEF])"; fi

# 10 (spec 301): the preflight is a judge run through the daemon, recorded before the real run starts, and its run is read back
D=$(fx judge succeeded); lane "$D"
PST=$(grep -n 'run start preflight' "$D/argv" | head -1 | cut -d: -f1)
RUNST=$(grep -n 'start-' "$D/argv" | head -1 | cut -d: -f1)
if [ -n "$PST" ] && [ -n "$RUNST" ] && [ "$PST" -lt "$RUNST" ] && grep -q 'run get r000000000418' "$D/argv"; then ok "preflight_runs_a_judge_through_the_daemon"; else bad "preflight_runs_a_judge_through_the_daemon (preflight start line [$PST], run start line [$RUNST])"; fi

# CONTROL 11 (spec 301): a preflight judge the daemon fails aborts before the real run starts
D=$(fx judge-fails succeeded "$TRACE_CRIT" 0 failed); lane "$D"
if [ "$(cat "$D/rc")" = 2 ] && grep -q 'ABORT: container preflight failed: the daemon.s judge cannot write' "$D/out" && grep -qF "$TRACE_CRIT" "$D/out" && ! grep -q 'start-' "$D/argv"; then ok "CONTROL_a_failed_preflight_judge_aborts"; else bad "CONTROL_a_failed_preflight_judge_aborts (rc [$(cat "$D/rc")])"; fi

# 12 (spec 301): the preflight graph is tracked with one judge node, and the spec lane copies it to the daemon's graph dir
rm -f "$REPO/_tmp/devbox-test/graphs/preflight.toml"
D=$(fx graph succeeded); lane "$D"
if [ -f graphs/preflight.toml ] && [ "$(grep -c 'kind = "judge"' graphs/preflight.toml)" = 1 ] && [ -f "$REPO/_tmp/devbox-test/graphs/preflight.toml" ]; then ok "preflight_graph_exists_and_is_copied"; else bad "preflight_graph_exists_and_is_copied"; fi

trash
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
