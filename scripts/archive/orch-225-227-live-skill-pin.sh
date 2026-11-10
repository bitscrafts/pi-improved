#!/usr/bin/env bash
# orch-225-227-live-skill-pin.sh — the ONE live reading that A94 (spec 225 criterion 6) and A95 (spec 227
# criterion 7) wait on, taken on the lnx host:
#
#   1. deploy HEAD to the lnx TEST daemon and read back its build revision;
#   2. put a skill into the daemon's own store THROUGH the daemon (`pii skill add` -> skill.put);
#   3. run a coder graph that pins that skill: `coder.spawned` must carry `skill=<name> sha256=<hex>`;
#   4. edit the document through the daemon (`pii skill edit` -> skill.edit) and demand that the SAME graph
#      now refuses with `skill.drift`, with no spawn at all.
#
# The journal is read as the daemon stores it: `store.redb` is snapshotted and dumped with the raw-event door
# (`pii graph trace <run> --db <snapshot>`). The `pii run trace` TABLE does not print the skill fields, so a
# check written against it would pass or fail on a display choice instead of on what the daemon journalled.
#
# Why a script (HARD RULE 0d): a deploy is the most expensive action here; it must be reviewable and re-runnable.
# Safety: only the lnx TEST daemon is touched — /run/user/1000/pii, binary /home/mvcorrea/.local/bin/pii — matched
# by comm==pii AND that exact argv, never by a command pattern (a pattern kill once stopped the containerised
# production daemon). The old binary is kept as pii.prev-<rev> and the script ROLLS BACK on its own if the new
# build does not answer `health` at the deployed revision.
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
REPO=/projects/pi-improved
HOST=mvcorrea@10.10.10.217
RD=/run/user/1000/pii
PII=$REPO/target/release/pii
REV=$(cd "$REPO" && git rev-parse --short=12 HEAD)
TS=$(date -u +%Y%m%dT%H%M%SZ)
SLUG="lint-live-$(printf '%s' "$TS" | tr 'A-Z' 'a-z')"
OUT=$REPO/_tmp/orch-225-227-live-$TS
mkdir -p "$OUT" || exit 1
say() { printf 'lnx-skill: %s\n' "$*" | tee -a "$OUT/reading.txt"; }
die() { printf 'lnx-skill: FAIL %s\n' "$*" | tee -a "$OUT/reading.txt"; exit 1; }
r() { ssh -o BatchMode=yes -o ConnectTimeout=8 "$HOST" "$@"; }
remote_pii() { r "PII_RUNTIME_DIR=$RD ~/.local/bin/pii $*"; }

say "revision $REV, skill name $SLUG, evidence $OUT"
[ -x "$PII" ] || die "no release binary at target/release/pii (build first)"

# ---------------------------------------------------------------- 1. deploy
say "local binary: $($PII --version 2>/dev/null | head -1)"
remote_rev() { remote_pii 'health --json 2>/dev/null' | python3 -c 'import sys,json
try: print(json.load(sys.stdin)["build"]["revision"])
except Exception: print("none")'; }
BEFORE=$(remote_rev)
say "lnx daemon revision before: $BEFORE"
restart() {
  r 'python3 - <<PY
import glob, os, signal, time
pid=None
for d in glob.glob("/proc/[0-9]*"):
    try:
        cmd=open(d+"/cmdline","rb").read(); comm=open(d+"/comm","rb").read()
    except Exception: continue
    if b"/home/mvcorrea/.local/bin/pii" in cmd and b"daemon" in cmd and comm.strip()==b"pii":
        pid=d.rsplit("/",1)[-1]; break
if pid:
    print("stopping daemon pid", pid); os.kill(int(pid), signal.SIGTERM); time.sleep(2)
else:
    print("no env-matched daemon found")
PY
export PII_RUNTIME_DIR=/run/user/1000/pii; export PII_MODEL="llamacpp-server:granite-4.2-3b"
mkdir -p $PII_RUNTIME_DIR
setsid nohup ~/.local/bin/pii daemon > ~/pii-daemon.log 2>&1 < /dev/null &
sleep 6; ls -l $PII_RUNTIME_DIR/control.sock | tail -1'
}
if [ "$BEFORE" = "$REV" ]; then
  say "deploy skipped: lnx already answers health at $REV"
else
  r "cp -p ~/.local/bin/pii ~/.local/bin/pii.prev-$BEFORE && ls -l ~/.local/bin/pii.prev-$BEFORE | awk '{print \$5}'" \
    || die "could not keep a backup of the running binary"
  scp -q "$PII" "$HOST:/home/mvcorrea/.local/bin/pii.new" || die "scp failed"
  r 'chmod +x ~/.local/bin/pii.new && mv -f ~/.local/bin/pii.new ~/.local/bin/pii' || die "install failed"
  restart >>"$OUT/restart.txt" 2>&1 || die "daemon restart failed (see restart.txt)"
  for _ in $(seq 1 15); do
    NOW=$(remote_rev); [ "$NOW" = "$REV" ] && break; sleep 2
  done
  if [ "$NOW" != "$REV" ]; then
    say "health reports '$NOW', expected '$REV' — rolling back to $BEFORE"
    r "mv -f ~/.local/bin/pii.prev-$BEFORE ~/.local/bin/pii" && restart >>"$OUT/restart.txt" 2>&1
    die "deployed build did not answer health at $REV; the previous binary was put back"
  fi
  say "PASS deploy: lnx daemon answers health at $REV"
fi

# ---------------------------------------------------------------- 2. skill into the store, through the daemon
printf 'PII-SKILL\n%s\n# lint\n\nRun the repository format and lint checks before proposing a change.\n' "$SLUG" >"$OUT/skill-v1.md"
printf 'PII-SKILL\n%s\n# lint\n\nRun the format, lint AND scratch-directory checks before proposing a change.\n' "$SLUG" >"$OUT/skill-v2.md"
scp -q "$OUT/skill-v1.md" "$HOST:/tmp/skill-pin-v1.md" || die "scp skill v1 failed"
ADD=$(remote_pii 'skill add --file /tmp/skill-pin-v1.md' 2>&1)
printf '%s\n' "$ADD" | tee -a "$OUT/reading.txt" >/dev/null
PIN=$(printf '%s' "$ADD" | sed -n 's/.*sha256=\([0-9a-f]\{64\}\).*/\1/p' | head -1)
SKID=$(printf '%s' "$ADD" | sed -n 's/^added \([^ ]*\) .*/\1/p' | head -1)
[ -n "$PIN" ] && [ -n "$SKID" ] || die "pii skill add did not print 'added <id> <slug> sha256=<hex>': $ADD"
say "PASS skill put through the daemon: id=$SKID name=$SLUG sha256=$PIN"

# ---------------------------------------------------------------- 3. a coder graph that pins it
r "mkdir -p /tmp/skill-pin-work /tmp/skill-pin-specs && printf '# live reading\npin the skill and spawn.\n' > /tmp/skill-pin-specs/spec.md" \
  || die "could not create the remote workdir/spec"
cat >"$OUT/coder-skill-pin.toml" <<EOF
# The live reading for spec 225 criterion 6 / spec 227 criterion 7: one coder node whose only skill is the one
# the store holds, pinned by the sha256 the daemon printed when the document was put.
name = "coder-skill-pin"
entry = "code"
caps = ["spawn", "proc"]

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
agent = "pi"
model = "skill-pin"
workdir = "/tmp/skill-pin-work"
spec = "/tmp/skill-pin-specs/spec.md"
brief = "Say one line and stop. This run measures that the pinned skill reaches the spawn."
env_pass = []
container_image = "localhost/pii-agent:1"
container_runtime = "socket"
timeout_ms = 120000
skills = [{ name = "$SLUG", sha256 = "$PIN" }]

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "refuse"
hook = "coder"
reason = "the agent failed"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
EOF
scp -q "$OUT/coder-skill-pin.toml" "$HOST:$RD/graphs/coder-skill-pin.toml" || die "scp graph failed"
say "graph installed at $RD/graphs/coder-skill-pin.toml"

journal() { # run_id -> raw journal events, from a snapshot of the daemon's own store
  scp -q "$HOST:$RD/store.redb" "$OUT/store-$1.redb" || return 1
  "$PII" graph trace "$1" --db "$OUT/store-$1.redb"
}

# ---------------------------------------------------------------- 4. the spawned run carries the pin
START=$(remote_pii 'run start coder-skill-pin' 2>&1)
RID=$(printf '%s' "$START" | grep -oE 'r[0-9]{12}' | head -1)
[ -n "$RID" ] || die "run start named no run id: $START"
say "run $RID started; waiting for the node"
sleep 20
journal "$RID" >"$OUT/journal-pinned.ndjson" 2>&1 || die "could not read the journal for $RID"
grep '"kind":"coder.spawned"' "$OUT/journal-pinned.ndjson" | head -1 | tee -a "$OUT/reading.txt" >/dev/null
grep -q "\"skill\":\"$SLUG\"" "$OUT/journal-pinned.ndjson" || die "coder.spawned does not carry skill=$SLUG"
grep -q "\"sha256\":\"$PIN\"" "$OUT/journal-pinned.ndjson" || die "coder.spawned does not carry sha256=$PIN"
say "PASS spawn: coder.spawned carries skill=$SLUG sha256=$PIN (runtime + container named in journal-pinned.ndjson)"

# ---------------------------------------------------------------- 5. edit the document, the pin must refuse
scp -q "$OUT/skill-v2.md" "$HOST:/tmp/skill-pin-v2.md" || die "scp skill v2 failed"
EDIT=$(remote_pii "skill edit $SKID --file /tmp/skill-pin-v2.md --note 'live reading 225/227'" 2>&1)
printf '%s\n' "$EDIT" | tee -a "$OUT/reading.txt" >/dev/null
say "edited through the daemon: $(printf '%s' "$EDIT" | head -1)"
START2=$(remote_pii 'run start coder-skill-pin' 2>&1)
RID2=$(printf '%s' "$START2" | grep -oE 'r[0-9]{12}' | head -1)
[ -n "$RID2" ] || die "second run start named no run id: $START2"
say "run $RID2 started after the edit; waiting"
sleep 10
journal "$RID2" >"$OUT/journal-drift.ndjson" 2>&1 || die "could not read the journal for $RID2"
grep '"kind":"skill.drift"' "$OUT/journal-drift.ndjson" | head -1 | tee -a "$OUT/reading.txt"
grep -q '"kind":"skill.drift"' "$OUT/journal-drift.ndjson" || die "the edited document did not refuse with skill.drift"
grep -q '"kind":"coder.spawned"' "$OUT/journal-drift.ndjson" && die "the drifted run SPAWNED anyway — the refusal came too late"
say "PASS drift: the edited document refuses with skill.drift and nothing spawned in $RID2"

# ---------------------------------------------------------------- 6. CONTROL: re-pin the edited text, it spawns again
# The refusal must be caused by the HASH, not by the edit having made the document unusable: pin what the drift
# reported as `found` and the same graph must reach a spawn carrying that new hash.
FOUND=$(grep -o '"found":"[0-9a-f]\{64\}"' "$OUT/journal-drift.ndjson" | head -1 | cut -d'"' -f4)
[ -n "$FOUND" ] || die "the drift event carried no found hash to re-pin"
sed -e "s/^name = \"coder-skill-pin\"/name = \"coder-skill-pin-repin\"/" \
    -e "s/$PIN/$FOUND/" "$OUT/coder-skill-pin.toml" >"$OUT/coder-skill-pin-repin.toml"
scp -q "$OUT/coder-skill-pin-repin.toml" "$HOST:$RD/graphs/coder-skill-pin-repin.toml" || die "scp re-pin graph failed"
START3=$(remote_pii 'run start coder-skill-pin-repin' 2>&1)
RID3=$(printf '%s' "$START3" | grep -oE 'r[0-9]{12}' | head -1)
[ -n "$RID3" ] || die "re-pin run start named no run id: $START3"
say "run $RID3 started with the re-pinned hash; waiting"
sleep 20
journal "$RID3" >"$OUT/journal-repin.ndjson" 2>&1 || die "could not read the journal for $RID3"
grep -q '"kind":"coder.spawned"' "$OUT/journal-repin.ndjson" || die "the re-pinned skill did not reach a spawn (control failed)"
grep -q "\"sha256\":\"$FOUND\"" "$OUT/journal-repin.ndjson" || die "the re-pinned spawn does not carry sha256=$FOUND"
grep -q '"kind":"skill.drift"' "$OUT/journal-repin.ndjson" && die "the re-pinned run still drifted — the refusal was not about the hash"
say "PASS control: re-pinned at $FOUND the same graph spawns and journals the new hash"
say "DONE — readings in $OUT"
