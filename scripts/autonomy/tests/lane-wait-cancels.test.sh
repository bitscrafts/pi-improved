#!/usr/bin/env bash
# exit 9 = refused before anything was spent (was 2, shared with a wrong command line; see run-batchC-pii.sh)
# requires: dev-tree
# why: it clones /projects/pi-improved by absolute path, so it only means anything in the dev tree
# Test: the spec lane waits as long as its graph allows, and cancels the run when it gives up (spec 299). The real lane script runs
# with a stub `ssh` first on PATH: it records every remote command, and answers from a fixture directory, so no daemon starts and nothing
# is spent. Each case runs a scratch copy of the script whose repo root is a scratch tree holding a copy of graphs/spec.toml.
# PII_POLL_SECONDS and PII_WAIT_MARGIN_S keep the waits short; the defaults (5 s, 600 s) are the production values.
set -uo pipefail
# the spec lane is chosen per case below; the variable is pinned here so a caller's environment cannot change a case
unset PII_LANE_GRAPH
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-wait-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; mv "$SCRATCH" "$REPO/_tmp/_trash/lane-wait-$(date +%s)"; }
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line

# the stub ssh: its last argument is the remote command; it answers from the fixture directory $STUB_FIX
mkdir -p "$SCRATCH/bin"
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/usr/bin/env bash
F=$STUB_FIX
echo "$*" >> "$F/argv"
cmd="${*: -1}"
case "$cmd" in
  *"mkdir -p /projects/_tmp"*) echo "preflight stub"; exit 0 ;;
  *"run start preflight"*) echo "run r000000000418 started" ;;
  *"run get r000000000418"*) echo '{"status":"succeeded"}' ;;
  *"run get"*)
    n=$(( $(cat "$F/polls" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$F/polls"
    s=$(cat "$F/status"); [ "$n" -ge "$(cat "$F/succeed_at" 2>/dev/null || echo 999999)" ] && s=succeeded
    echo "{\"status\":\"$s\"}" ;;
  *"run cancel"*) echo "cancel stub: cancelled" ;;
  *"run trace"*) ;;
  *"run explain"*) echo "explain stub" ;;
  *"start-"*|*"run start"*) echo "run r000000000419 started" ;;
  *"coder.spawned"*) echo '{"kind":"coder.spawned","session":"sid-299"}' ;;
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

# mkrepo <name> <wall line: a value | none>: a scratch repo root with the lane script, the real spec and docs graph, and a spec.toml
# whose max_wall_ms is the given value (or the line removed); the daemon is stubbed to come up at once
mkrepo() {
  local d=$SCRATCH/$1 W=$2
  mkdir -p "$d/scripts" "$d/graphs" "$d/specs" "$d/modules/devbox"
  cp "$REPO"/specs/299-*.md "$d/specs/"
  cp "$REPO/graphs/docs.toml" "$REPO/graphs/preflight.toml" "$d/graphs/"
  if [ "$W" = none ]; then sed '/^max_wall_ms/d' "$REPO/graphs/spec.toml" > "$d/graphs/spec.toml"
  else sed "s/^max_wall_ms = .*/max_wall_ms = $W/" "$REPO/graphs/spec.toml" > "$d/graphs/spec.toml"; fi
  sed "s|^REPO=/projects/pi-improved|REPO=$d|" "$REPO/scripts/run-batchC-pii.sh" > "$d/scripts/run-batchC-pii.sh"
  printf '#!/usr/bin/env bash\necho "daemon up (test)"\n' > "$d/modules/devbox/daemon.sh"
  echo "$d"
}
# fx <name> <status> [succeed_at]: a fresh fixture directory for the stub
fx() { local d=$SCRATCH/$1; mkdir -p "$d"; : > "$d/argv"; printf '%s\n' "$2" > "$d/status"; [ -n "${3:-}" ] && printf '%s\n' "$3" > "$d/succeed_at"; echo "$d"; }
# lane <repo> <fixture> [args...]: the lane script of that repo (spec lane unless LANE_GRAPH says otherwise); stdout+stderr in <fixture>/out, rc in <fixture>/rc
lane() {
  local r=$1 d=$2; shift 2
  if [ -n "${LANE_GRAPH-spec}" ]; then L="PII_LANE_GRAPH=${LANE_GRAPH-spec}"; else L="-u PII_LANE_GRAPH"; fi
  STUB_FIX=$d PII_DAEMON_SH="$SCRATCH/daemon.sh" PII_SSH="$SCRATCH/bin/ssh" PATH="$SCRATCH/bin:$PATH" env $L timeout 60 bash "$r/scripts/run-batchC-pii.sh" 299 "$@" > "$d/out" 2>&1 < /dev/null
  echo $? > "$d/rc"
}

# 1: the WAIT line takes the deadline from the graph's own wall: 7000000 ms + 600 s
R=$(mkrepo wall7000 7000000); D=$(fx dry running); LANE_GRAPH=spec lane "$R" "$D" --dry-run
if grep -q '^WAIT     : up to 7600s ' "$D/out"; then ok "deadline_comes_from_the_graph_wall"; else bad "deadline_comes_from_the_graph_wall ($(field "$(cat "$D/out")" WAIT))"; fi

# CONTROL 2: a graph without a numeric wall aborts with exit 2, and nothing is started
R=$(mkrepo nowall none); D=$(fx nowall running); LANE_GRAPH=spec lane "$R" "$D"
if [ "$(cat "$D/rc")" = 9 ] && grep -q 'no numeric max_wall_ms' "$D/out" && ! grep -q 'start-' "$D/argv"; then ok "CONTROL_a_graph_without_a_wall_aborts"; else bad "CONTROL_a_graph_without_a_wall_aborts (rc [$(cat "$D/rc")])"; fi

# 3: a run that outlives the deadline is cancelled with --kill and the reason, and the lane exits 1
R=$(mkrepo wall2000 2000); D=$(fx outlives running)
LANE_GRAPH=spec PII_POLL_SECONDS=1 PII_WAIT_MARGIN_S=0 lane "$R" "$D"
LOGF=$(field "$(cat "$D/out")" LOG)
if grep -q 'run cancel r000000000419 --kill --reason lane-wait-expired' "$D/argv" && [ "$(cat "$D/rc")" = 1 ] \
  && grep -q 'cancelled by the lane' "$D/out" && grep -q 'cancelled by the lane' "$LOGF" && grep -q 'cancel stub: cancelled' "$LOGF"; then
  ok "a_run_that_outlives_the_deadline_is_cancelled"
else bad "a_run_that_outlives_the_deadline_is_cancelled (rc [$(cat "$D/rc")], log [$LOGF])"; fi

# CONTROL 4: a run that succeeds before the deadline is not cancelled
R=$(mkrepo wall2000b 2000); D=$(fx finishes running 2); LANE_GRAPH=spec PII_POLL_SECONDS=1 PII_WAIT_MARGIN_S=0 lane "$R" "$D"
if ! grep -q 'run cancel' "$D/argv" && [ "$(cat "$D/rc")" = 0 ]; then ok "CONTROL_a_run_that_finishes_is_not_cancelled"; else bad "CONTROL_a_run_that_finishes_is_not_cancelled (rc [$(cat "$D/rc")])"; fi

# CONTROL 5: the coder lane keeps its iteration bound and never cancels
R=$(mkrepo coder 6000000); D=$(fx coder running); LANE_GRAPH= PII_NODE_TIMEOUT_MS=1000 PII_POLL_SECONDS=0 lane "$R" "$D"
if ! grep -q 'run cancel' "$D/argv" && [ "$(cat "$D/rc")" = 1 ]; then ok "CONTROL_the_coder_lane_never_cancels"; else bad "CONTROL_the_coder_lane_never_cancels (rc [$(cat "$D/rc")])"; fi

trash
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
