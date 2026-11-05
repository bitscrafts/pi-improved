#!/usr/bin/env bash
# requires: dev-tree
# why: it clones /projects/pi-improved by absolute path, so it only means anything in the dev tree
# Test: a run that FAILED without any coder node ever running was refused before a worker was bought, so the lane
# exits 9 (refused, nothing spent), not 1 (a worker that tried). Measured 2026-11-05: `sdd`'s contract judge refused
# eight rows whose exit criteria already passed; the lane recorded each as exit 1, the cycle as `worker-rc=1`, and
# the queue read as if a worker had failed eight times. The real lane script runs with a stub `ssh` that answers
# from a fixture directory, so no daemon starts and nothing is spent.
set -uo pipefail
unset PII_LANE_GRAPH
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-refused-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; mv "$SCRATCH" "$REPO/_tmp/_trash/lane-refused-$(date +%s)"; }

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
  *"run get"*) echo "{\"status\":\"$(cat "$F/status")\"}" ;;
  *"run cancel"*) echo "cancel stub: cancelled" ;;
  *"run trace"*) cat "$F/trace" ;;
  *"run explain"*) echo "explain stub" ;;
  *"start-"*|*"run start"*) echo "run r000000000419 started" ;;
  *"coder.spawned"*) echo '{"kind":"coder.spawned","session":"sid-refused"}' ;;
  *"coder.exited"*) [ -f "$F/coder_ran" ] && echo '{"kind":"coder.exited","rc":"1","tree_changed":"no","quota_exhausted":"no"}' ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/ssh"
printf '#!/usr/bin/env bash\necho '"'"'"revision":"stub000000000"'"'"'\n' > "$SCRATCH/daemon.sh"; chmod +x "$SCRATCH/daemon.sh"

mkrepo() {
  local d=$SCRATCH/$1
  mkdir -p "$d/scripts" "$d/graphs" "$d/specs" "$d/modules/devbox"
  cp "$REPO"/specs/299-*.md "$d/specs/"
  cp "$REPO/graphs/docs.toml" "$REPO/graphs/preflight.toml" "$REPO/graphs/spec.toml" "$d/graphs/"
  while read -r g; do case "$g" in ''|'#'*) continue ;; esac; [ -f "$REPO/graphs/$g.toml" ] && cp "$REPO/graphs/$g.toml" "$d/graphs/"; done < "$REPO/graphs/minimal.txt"
  cp "$REPO/graphs/minimal.txt" "$d/graphs/"
  sed "s|^REPO=/projects/pi-improved|REPO=$d|" "$REPO/scripts/run-batchC-pii.sh" > "$d/scripts/run-batchC-pii.sh"
  printf '#!/usr/bin/env bash\necho "daemon up (test)"\n' > "$d/modules/devbox/daemon.sh"
  (cd "$d" && git init -q . && git add -A && git -c user.email=t@t -c user.name=t commit -qm base)
  echo "$d"
}
# fx <name> <status> <trace-file-content> [coder_ran]
fx() { local d=$SCRATCH/$1; mkdir -p "$d"; : > "$d/argv"; printf '%s\n' "$2" > "$d/status"; printf '%s\n' "$3" > "$d/trace"; [ -n "${4:-}" ] && : > "$d/coder_ran"; echo "$d"; }
lane() {
  local r=$1 d=$2
  (cd "$r" && STUB_FIX=$d PII_DAEMON_SH="$SCRATCH/daemon.sh" PII_SSH="$SCRATCH/bin/ssh" PATH="$SCRATCH/bin:$PATH" \
    PII_LANE_GRAPH=spec PII_GRAPH_EDIT_OK=1 PII_POLL_SECONDS=0 timeout 120 bash "$r/scripts/run-batchC-pii.sh" 299 > "$d/out" 2>&1)
  echo $? > "$d/rc"
}

NOCODER='52 sdd__contract - 55+60 pid=- image=- crit=0/4 fail=- rc=-
80 sdd__refused - 122+0 pid=- image=- crit=- fail=- rc=-
83 refused - 127+- pid=- image=- crit=- fail=run.refused{hook=coder, reason=the agent failed: expect fail: criterion passed: test -f x (rc 0)} rc=-'
WITHCODER='52 sdd__contract - 55+60 pid=- image=- crit=0/4 fail=- rc=-
61 sdd__tdd_write_test coder 379+281095 pid=2035107 image=- crit=0/0 fail=- rc=0
83 refused - 300+- pid=- image=- crit=- fail=run.refused{hook=judge, reason=the implementation did not make the test pass} rc=-'

# 1: failed, and no coder node ever ran -> refused before spending: exit 9, and it says so
R=$(mkrepo refused-none); D=$(fx none failed "$NOCODER"); lane "$R" "$D"
if [ "$(cat "$D/rc")" = 9 ] && grep -q 'REFUSED  : the run ended before any coder ran' "$D/out"; then ok "a_run_refused_before_any_coder_exits_9"
else bad "a_run_refused_before_any_coder_exits_9 (rc [$(cat "$D/rc")])"; fi

# CONTROL 2: failed AFTER a coder ran (a genuine attempt, 695 s in the field) -> still exit 1
R=$(mkrepo refused-coder); D=$(fx coder failed "$WITHCODER" yes); lane "$R" "$D"
if [ "$(cat "$D/rc")" = 1 ] && ! grep -q 'REFUSED  :' "$D/out"; then ok "CONTROL_a_run_that_reached_a_coder_still_exits_1"
else bad "CONTROL_a_run_that_reached_a_coder_still_exits_1 (rc [$(cat "$D/rc")])"; fi

# CONTROL 3: a run that SUCCEEDED is untouched
R=$(mkrepo refused-ok); D=$(fx ok succeeded "$WITHCODER" yes); lane "$R" "$D"
if [ "$(cat "$D/rc")" = 0 ] && ! grep -q 'REFUSED  :' "$D/out"; then ok "CONTROL_a_run_that_succeeds_exits_0"
else bad "CONTROL_a_run_that_succeeds_exits_0 (rc [$(cat "$D/rc")])"; fi

# CONTROL 4: a run still running at the deadline is CANCELLED and exits 1 — it spent, so it is not a refusal
R=$(mkrepo refused-timeout); D=$(fx timeout running ""); sed -i 's/^max_wall_ms = .*/max_wall_ms = 2000/' "$R/graphs/spec.toml"; (cd "$R" && git add -A && git -c user.email=t@t -c user.name=t commit -qm wall)
PII_WAIT_MARGIN_S=0 PII_POLL_SECONDS=1 lane "$R" "$D"
if [ "$(cat "$D/rc")" = 1 ] && ! grep -q 'REFUSED  :' "$D/out"; then ok "CONTROL_a_timeout_is_not_a_refusal"
else bad "CONTROL_a_timeout_is_not_a_refusal (rc [$(cat "$D/rc")])"; fi

trash
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
