#!/usr/bin/env bash
# Test: a quota refusal by EITHER coder of the spec lane reaches the worker log, where window.sh reads it (spec 300, F3). The real lane
# script runs with a stub `ssh` first on PATH: it answers from a fixture directory, so no daemon starts and nothing is spent. Each case
# runs a scratch copy of the script whose repo root is a scratch tree. PII_POLL_SECONDS and PII_WAIT_MARGIN_S keep the waits short.
set -uo pipefail
# the spec lane is pinned here so a caller's environment cannot change a case
unset PII_LANE_GRAPH
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-facts-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; mv "$SCRATCH" "$REPO/_tmp/_trash/lane-facts-$(date +%s)"; }
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line

# the stub ssh: its last argument is the remote command; it answers from the fixture directory $STUB_FIX. The journal's coder.exited
# lines are the fixture's `exits` file: the host's first, the docs coder's second (the spec graph runs two coders).
mkdir -p "$SCRATCH/bin"
# window.sh parses with python: this box may have only python3.13 (under /root/.local/bin), so the test links whichever interpreter it finds
PY=$(command -v python3 || command -v python3.13) || { echo "FAIL setup: no python3 on PATH"; exit 2; }
ln -s "$PY" "$SCRATCH/bin/python3"
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/usr/bin/env bash
F=$STUB_FIX
echo "$*" >> "$F/argv"
cmd="${*: -1}"
case "$cmd" in
  *"mkdir -p /projects/_tmp"*) echo "preflight stub"; exit 0 ;;
  *"run get"*) echo '{"status":"succeeded"}' ;;
  *"run explain"*) echo "explain stub" ;;
  *"run trace"*) ;;
  *"start-"*|*"run start"*) echo "run r000000000420 started" ;;
  *"coder.spawned"*) echo '{"kind":"coder.spawned","session":"sid-300"}' ;;
  *"coder.exited"*) cat "$F/exits" ;;
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

# mkrepo <name>: a scratch repo root with the lane script, the real spec 300, the real spec and docs graphs, and a daemon stub that comes up at once
mkrepo() {
  local d=$SCRATCH/$1
  mkdir -p "$d/scripts" "$d/graphs" "$d/specs" "$d/modules/devbox"
  cp "$REPO"/specs/300-*.md "$d/specs/"
  cp "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" "$d/graphs/"
  sed "s|^REPO=/projects/pi-improved|REPO=$d|" "$REPO/scripts/run-batchC-pii.sh" > "$d/scripts/run-batchC-pii.sh"
  printf '#!/usr/bin/env bash\necho "daemon up (test)"\n' > "$d/modules/devbox/daemon.sh"
  echo "$d"
}
# fx <name> <exits-json-lines>: a fresh fixture directory for the stub, with the coder.exited lines the journal would hold
fx() { local d=$SCRATCH/$1; mkdir -p "$d"; : > "$d/argv"; printf '%s\n' "$2" > "$d/exits"; echo "$d"; }
# lane <repo> <fixture>: the spec lane script of that repo; stdout+stderr in <fixture>/out, rc in <fixture>/rc
lane() {
  local r=$1 d=$2
  STUB_FIX=$d PII_DAEMON_SH="$SCRATCH/daemon.sh" PII_SSH="$SCRATCH/bin/ssh" PATH="$SCRATCH/bin:$PATH" env PII_LANE_GRAPH=spec PII_POLL_SECONDS=0 PII_WAIT_MARGIN_S=0 bash "$r/scripts/run-batchC-pii.sh" 300 > "$d/out" 2>&1 < /dev/null
  echo $? > "$d/rc"
}
# the lines the journal holds for the two coders of one run (rc, tree_changed, quota facts)
HOST_OK='{"kind":"coder.exited","rc":"0","tree_changed":"yes","quota_exhausted":"no","quota_reset":"resets 3:05am (UTC)"}'
DOCS_OK='{"kind":"coder.exited","rc":"0","tree_changed":"no","quota_exhausted":"no"}'
DOCS_REFUSED='{"kind":"coder.exited","rc":"1","tree_changed":"no","quota_exhausted":"yes","quota_reset":"resets 8:20pm (UTC)"}'
HOST_REFUSED='{"kind":"coder.exited","rc":"1","tree_changed":"no","quota_exhausted":"yes","quota_reset":"resets 8:20pm (UTC)"}'
PHRASE_ONLY='{"kind":"coder.exited","rc":"0","tree_changed":"no","quota_exhausted":"no","text":"You'"'"'ve hit your session limit · resets 8:20pm (UTC)"}'
LINE="You've hit your session limit · resets 8:20pm (UTC)"

# 1: the docs coder's refusal is in the worker log, with the docs coder's own reset, and window.sh reads an epoch from that log
R=$(mkrepo docs); D=$(fx docs "$HOST_OK
$DOCS_REFUSED"); lane "$R" "$D"
LOGF=$(field "$(cat "$D/out")" LOG); EP=$(PATH="$SCRATCH/bin:$PATH" bash "$REPO/scripts/autonomy/window.sh" parse "$LOGF" 2>/dev/null)
if grep -qxF "$LINE" "$LOGF" && [[ "$EP" =~ ^[0-9]+$ ]] && grep -qxF 'QUOTA    : refused by the docs coder' "$D/out"; then
  ok "docs_quota_refusal_reaches_the_worker_log"
else bad "docs_quota_refusal_reaches_the_worker_log (log [$LOGF], epoch [$EP], rc [$(cat "$D/rc")])"; fi

# 2: the host coder's refusal still reaches the log, named as the host's
R=$(mkrepo host); D=$(fx host "$HOST_REFUSED
$DOCS_OK"); lane "$R" "$D"
LOGF=$(field "$(cat "$D/out")" LOG)
if grep -qxF "$LINE" "$LOGF" && grep -qxF 'QUOTA    : refused by the host coder' "$D/out"; then
  ok "host_quota_refusal_still_reaches_the_log"
else bad "host_quota_refusal_still_reaches_the_log (log [$LOGF])"; fi

# CONTROL 3: no refusal by either coder writes no quota line
R=$(mkrepo none); D=$(fx none "$HOST_OK
$DOCS_OK"); lane "$R" "$D"
LOGF=$(field "$(cat "$D/out")" LOG)
if ! grep -q 'hit your session limit' "$LOGF" && ! grep -q '^QUOTA' "$D/out"; then ok "CONTROL_no_refusal_writes_no_quota_line"
else bad "CONTROL_no_refusal_writes_no_quota_line (log [$LOGF])"; fi

# CONTROL 4: a docs coder whose output only contains the phrase, with quota_exhausted "no", is not a refusal
R=$(mkrepo phrase); D=$(fx phrase "$HOST_OK
$PHRASE_ONLY"); lane "$R" "$D"
LOGF=$(field "$(cat "$D/out")" LOG)
if ! grep -q 'hit your session limit' "$LOGF" && ! grep -q '^QUOTA' "$D/out"; then ok "CONTROL_a_docs_coder_that_only_read_the_phrase_is_not_a_refusal"
else bad "CONTROL_a_docs_coder_that_only_read_the_phrase_is_not_a_refusal (log [$LOGF])"; fi

trash
echo "PASS: $PASS FAIL: $FAIL"
[ "$FAIL" -eq 0 ]
