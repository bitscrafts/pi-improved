#!/usr/bin/env bash
# deploy-lnx-test.sh — build here, install on the lnx HOST (which has podman), run the coder graph there, print the reading.
#
# Why a script (HARD RULE 0d): a dispatch/deploy is the most expensive action; it must be reviewable and re-runnable, and it
# must set its own PATH — three deploys tonight died on "cargo: command not found" or a stale binary.
#
# Safety: the lnx daemon is restarted by PID FOUND THROUGH ITS ENVIRONMENT (PII_RUNTIME_DIR=/run/user/1000/pii), never by a
# command pattern — a pattern kill earlier tonight matched the containerised production daemon and stopped it.
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
REPO=/projects/pi-improved
HOST=mvcorrea@10.10.10.217
RD=/run/user/1000/pii
step() { printf 'lnx-test: %s\n' "$*"; }
fail() { printf 'lnx-test: FAIL %s\n' "$*"; exit 1; }

cd "$REPO" || exit 1
REV=$(git rev-parse --short=12 HEAD)
step "revision $REV"

step "building the release binaries here"
cargo build --release -p pii-cli >/tmp/lnx-build.log 2>&1 || fail "build failed (see /tmp/lnx-build.log)"
step "build ok: $(ls -l target/release/pii | awk '{print $5}') bytes"

# Copy to a NEW name and mv it into place: scp straight onto a binary that is being executed fails with
# "dest open ... Failure" (measured 2026-10-20). mv replaces the inode, which a running process tolerates.
step "copying pii to $HOST:~/.local/bin/pii.new then mv into place"
scp -q "$REPO/target/release/pii" "$HOST:/home/mvcorrea/.local/bin/pii.new" || fail "scp failed"
ssh -o BatchMode=yes "$HOST" 'chmod +x ~/.local/bin/pii.new && mv -f ~/.local/bin/pii.new ~/.local/bin/pii && ls -l ~/.local/bin/pii | awk "{print \$5}"' || fail "install (mv) failed"

step "placing graphs + fragment in the runtime dir the daemon actually reads"
ssh -o BatchMode=yes "$HOST" "mkdir -p $RD/graphs/fragments && cp ~/.local/state/pii/graphs/*.toml $RD/graphs/ 2>/dev/null; \
  cp ~/.local/state/pii/graphs/fragments/*.toml $RD/graphs/fragments/ 2>/dev/null; echo placed" || fail "graph copy failed"

step "restarting the lnx daemon by PID (env-matched, never by pattern)"
ssh -o BatchMode=yes "$HOST" 'python3 - <<PY
import glob, os, signal, time
pid=None
for d in glob.glob("/proc/[0-9]*"):
    try:
        env=open(d+"/environ","rb").read()
        cmd=open(d+"/cmdline","rb").read()
    except Exception: continue
    if b"pii daemon" in cmd and b"PII_RUNTIME_DIR=/run/user/1000/pii" in env:
        pid=d.rsplit("/",1)[-1]; break
if pid:
    print("stopping daemon pid", pid); os.kill(int(pid), signal.SIGTERM); time.sleep(2)
else:
    print("no env-matched daemon found")
PY
export PII_RUNTIME_DIR=/run/user/1000/pii; export PII_MODEL="llamacpp-server:granite-4.2-3b"
mkdir -p $PII_RUNTIME_DIR
setsid nohup ~/.local/bin/pii daemon > ~/pii-daemon.log 2>&1 < /dev/null &
sleep 6; ls -l $PII_RUNTIME_DIR/control.sock | tail -1' || fail "daemon restart failed"

step "READING build: $(ssh -o BatchMode=yes "$HOST" "PII_RUNTIME_DIR=$RD ~/.local/bin/pii health --json" 2>/dev/null | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d["build"]["revision"])' 2>/dev/null)"

step "TEST: run start coder-podman"
OUT=$(ssh -o BatchMode=yes "$HOST" "PII_RUNTIME_DIR=$RD ~/.local/bin/pii run start coder-podman" 2>&1 | tail -3)
printf '%s\n' "$OUT" | sed 's/^/lnx-test:   /'
RID=$(printf '%s' "$OUT" | grep -oE 'r[0-9]{12}' | head -1)
[ -n "$RID" ] || fail "no run id"
sleep 20
step "run get"
ssh -o BatchMode=yes "$HOST" "PII_RUNTIME_DIR=$RD ~/.local/bin/pii run get $RID" 2>&1 | head -c 300; echo
step "trace for $RID"
ssh -o BatchMode=yes "$HOST" "PII_RUNTIME_DIR=$RD ~/.local/bin/pii run trace $RID" 2>&1 | tail -14
step "DONE"
