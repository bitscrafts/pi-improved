#!/usr/bin/env bash
# scripts/checks/resume-through-daemon.sh — spec 307 R2: a session a daemon's coder starts can be resumed, with a REAL claude (haiku).
# A PRIVATE daemon (its own runtime dir, store and graph dir; the production daemon is never touched) runs a one-node coder that is asked
# "Reply with exactly: PONG". Its session id comes from the journal's coder.spawned. A second run resumes that session with a question only
# the first run can answer; the answer (coder.exited's agent_output_head) must contain PONG.
# CONTROL: the same two steps on a daemon WITHOUT CLAUDE_CONFIG_DIR (the login is still mounted) must FAIL with "No conversation found".
# Costs a few cents. Needs the devbox (ssh + podman), as scripts/run-batchC-pii.sh does.
# Output ends with RESUME-THROUGH-DAEMON: OK, or RESUME-THROUGH-DAEMON: FAIL <why>. Exit 0 only on OK.
set -uo pipefail
REPO=/projects/pi-improved
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
SSH=("${PII_SSH:-ssh}" -o BatchMode=yes -o ConnectTimeout=15 "$HOST")
C=$REPO/_tmp/devbox-test
PII=$C/bin/pii                  # the daemon's own binary copy (daemon.sh refresh); this check never rebuilds it
P=$REPO/_tmp/resume-check       # the private state: one dir per daemon, the claude config dir, the spec
SPECREL=_tmp/resume-check/spec.md
# Inside the devbox (where the spec judge runs this) the login mount exists and there is no ssh/podman: run the command here. On the host, go through ssh + podman exec.
INSIDE=""; [ -d /run/claude-login ] && INSIDE=1
X() { if [ -n "$INSIDE" ]; then (cd /projects/pi-improved && eval "$*"); else "${SSH[@]}" "podman exec -w /projects/pi-improved pii-devbox $*"; fi; }
XD() { if [ -n "$INSIDE" ]; then (cd /projects/pi-improved && nohup sh "$1" >/dev/null 2>&1 &); else "${SSH[@]}" "podman exec -d -w /projects/pi-improved pii-devbox sh $1"; fi; }
health() { X "sh -c 'env PII_RUNTIME_DIR=$P/$1/run $PII health --json 2>/dev/null'" | head -c 400; }
stop_daemon() { X "sh -c 'kill \$(cat $P/$1/daemon.pid) 2>/dev/null; true'"; }
trap 'stop_daemon test; stop_daemon control' EXIT

# start_daemon <name> <CLAUDE_CONFIG_DIR, or empty for none>: started the way modules/devbox/daemon.sh starts the production one
start_daemon() {
  local D=$P/$1 CFG_ENV=""
  [ -n "$2" ] && CFG_ENV="CLAUDE_CONFIG_DIR=$2"
  mkdir -p "$D/run" "$D/graphs" "$P/claude" && rm -f "$D/run/control.sock" \
    && ln -sfn /run/claude-login/.credentials.json "$P/claude/.credentials.json" || return 1
  printf '#!/bin/sh\necho $$ > %s/daemon.pid\nexec env PII_RUNTIME_DIR=%s/run %s PI_CODING_AGENT_DIR=/projects/pi-improved/_tmp/pii-agent IS_SANDBOX=1 PII_MODEL=llamacpp-server:granite-4.2-3b CLAUDE_CODE_TMPDIR=%s/tmp TMPDIR=%s/tmp %s daemon --foreground --store %s/store.redb --workspace %s --log-file %s/pii.log > %s/daemon.out 2>&1\n' \
    "$D" "$D" "$CFG_ENV" "$D" "$D" "$PII" "$D" "$REPO" "$D" "$D" > "$P/start-$1.sh"
  XD "$P/start-$1.sh" || return 1
  for _ in $(seq 1 30); do [ -n "$(health "$1")" ] && return 0; sleep 1; done
  echo "private daemon $1 did not answer in 30 s"; return 1
}

# run_coder <daemon> <graph name> <sid, or empty> <brief>: one coder run through the private daemon. Sets RUN_SID (the spawn's session),
# RUN_STATUS and RUN_EXITED (the run's coder.exited line, the journal's answer and failure text).
run_coder() {
  local D=$P/$1 NAME=$2 SID=$3 BRIEF=$4 LINES RID i
  { echo "name = \"$NAME\""; echo 'entry = "code"'; echo 'caps = ["spawn", "proc"]'; echo; echo '[limits]'; echo 'max_wall_ms = 600000'; echo;
    echo '[[node]]'; echo 'id = "code"'; echo 'kind = "coder"'; echo 'caps = ["spawn", "proc"]'; echo 'agent = "claude"'
    echo 'model = "claude-haiku-5-5"'; echo 'max_budget_usd = 0.25'; [ -n "$SID" ] && echo "sid = \"$SID\""
    echo "workdir = \"$REPO\""; echo "spec = \"$SPECREL\""; echo "brief = '''"; echo "$BRIEF"; echo "'''"
    echo 'env_pass = ["PATH", "HOME", "IS_SANDBOX", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR", "CLAUDE_CONFIG_DIR"]'
    echo 'timeout_ms = 300000'; echo; echo '[[node]]'; echo 'id = "done"'; echo 'kind = "transform"'; echo 'value = "done"'; echo;
    echo '[[node]]'; echo 'id = "refused"'; echo 'kind = "refuse"'; echo 'hook = "coder"'; echo 'reason = "the agent failed"'; echo;
    echo '[[edge]]'; echo 'from = "code"'; echo 'port = "ok"'; echo 'to = "done"'; echo 'kind = "data"'; echo;
    echo '[[edge]]'; echo 'from = "code"'; echo 'port = "err"'; echo 'to = "refused"'; echo 'kind = "data"'; } > "$D/graphs/$NAME.toml"
  LINES=$(X "sh -c 'cat $D/pii.log 2>/dev/null | wc -l'" | tr -d ' ')
  RID=$(X "sh -c 'env PII_RUNTIME_DIR=$D/run $PII run start $NAME 2>&1'" | grep -o 'r[0-9]\{12\}' | head -1)
  [ -n "$RID" ] || { echo "no run id for $NAME"; return 1; }
  RUN_SID=""
  for i in $(seq 1 60); do
    RUN_SID=$(X "sh -c 'tail -n +$((LINES+1)) $D/pii.log | grep -a coder.spawned | tail -1'" | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
    [ -n "$RUN_SID" ] && break; sleep 2
  done
  for i in $(seq 1 120); do
    RUN_STATUS=$(X "sh -c 'env PII_RUNTIME_DIR=$D/run $PII run get $RID 2>&1'" | grep -o '"status":"[a-z_]*"' | head -1)
    case "$RUN_STATUS" in *succeeded*|*failed*|*refused*|*cancelled*) break ;; esac
    sleep 5
  done
  RUN_EXITED=$(X "sh -c 'tail -n +$((LINES+1)) $D/pii.log | grep -a coder.exited | tail -1'")
  echo "ROW $1 $NAME: run $RID ${RUN_STATUS:-no status}, session ${RUN_SID:-none}"
}

mkdir -p "$P" && printf '# Spec: resume check\n\nReply with exactly the word asked for. Change no file.\n' > "$P/spec.md"
ASK='What single word did I ask you to reply with?'

# ---- the real check: a session the private daemon's coder started, resumed by a second run
start_daemon test "$P/claude" || { echo "RESUME-THROUGH-DAEMON: FAIL the private daemon did not start"; exit 1; }
run_coder test resume-first "" "Reply with exactly: PONG"
SID=$RUN_SID
[ -n "$SID" ] || { echo "RESUME-THROUGH-DAEMON: FAIL the first run journalled no coder.spawned session"; exit 1; }
run_coder test resume-second "$SID" "$ASK"
if printf '%s' "$RUN_EXITED" | grep -q 'PONG'; then echo "ROW test: step 2 resumed session $SID and answered PONG"
else echo "RESUME-THROUGH-DAEMON: FAIL step 2 did not answer PONG: $(printf '%s' "$RUN_EXITED" | head -c 300)"; exit 1; fi
stop_daemon test

# ---- CONTROL: the same two steps with CLAUDE_CONFIG_DIR unset. Step 2 must fail with "No conversation found", or the check proves nothing.
start_daemon control "" || { echo "RESUME-THROUGH-DAEMON: FAIL the control daemon did not start"; exit 1; }
run_coder control control-first "" "Reply with exactly: PONG"
CSID=$RUN_SID
run_coder control control-second "$CSID" "$ASK"
stop_daemon control
if printf '%s' "$RUN_EXITED" | grep -q 'No conversation found'; then
  echo "CONTROL (CLAUDE_CONFIG_DIR unset): step 2 FAIL: No conversation found with session ID ${CSID:-none}"
else
  echo "CONTROL INVALID: $(printf '%s' "$RUN_EXITED" | head -c 300)"; echo "RESUME-THROUGH-DAEMON: FAIL CONTROL INVALID"; exit 1
fi
echo "RESUME-THROUGH-DAEMON: OK"
