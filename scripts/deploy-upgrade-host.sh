#!/usr/bin/env bash
# deploy-upgrade-host.sh <rev> [--apply] — upgrade the running pii on the lnx host to a gated commit, in place, reversibly.
#
#   bash scripts/deploy-upgrade-host.sh a92301f            dry run: checks everything, prints the plan, changes nothing
#   bash scripts/deploy-upgrade-host.sh a92301f --apply    does it
#
# Why this and not deploy/install-pii.sh: the host already runs a native per-user unit (deploy/install-pii-user.sh) with config
# under ~/.local/pii; install-pii.sh would write ~/.config/pii and a second unit. This UPGRADES what is running and touches
# nothing else: the binary, the shipped graphs, one [models] block, and nothing of the stores, the credentials or the unit file.
#
# Order, and why: (1) every precondition, before anything moves; (2) STOP the daemon, because the stores are single-writer
# and a copy of an open redb is not a backup; (3) back up binary + config + graphs + stores to a dated dir, (4) swap, (5) start,
# (6) health must name the new revision. A failure after step 2 restores the backup and restarts the OLD binary, on its own.
# Never deletes: replaced files are kept in the backup dir. Secrets are never read or printed (only variable NAMES are shown).
set -uo pipefail
export PATH=/usr/local/cargo/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin:$PATH
REPO=/projects/pi-improved
HOST=${HOST:-mvcorrea@10.10.10.217}
REV=${1:-}; APPLY=0; [ "${2:-}" = "--apply" ] && APPLY=1
# REMOTE_ROOT: the host account's home. Rehearsals set it to a scratch dir and NO_UNIT=1 (no systemd, no real daemon touched).
REMOTE_ROOT=${REMOTE_ROOT:-}; NO_UNIT=${NO_UNIT:-0}
say(){ printf '%s\n' "$*"; }
refuse(){ say "REFUSED (nothing was changed): $*" >&2; exit 2; }
sshx(){ timeout 180 ssh -o BatchMode=yes "$HOST" "$@"; }

[ -n "$REV" ] || refuse "usage: $0 <rev> [--apply]"
REV12=$(git -C "$REPO" rev-parse --verify --short=12 "$REV^{commit}") || refuse "$REV is not a commit"
REV7=${REV12:0:7}
REL=/projects/_tmp/release-$REV12
CARD=$REPO/_tmp/deploy/card-$REV7.json
GATELOG=$REPO/_tmp/fresh-gate-head.log

say "== 1. preconditions"
[ -x "$REL/pii" ] || refuse "no release build at $REL/pii: run scripts/build-release-for-host.sh $REV12"
( cd "$REL" && sha256sum -c pii.sha256 >/dev/null 2>&1 ) || refuse "$REL/pii does not match its recorded checksum"
LABEL=$(strings -n 12 "$REL/pii" | grep -oE "$REV12(-dirty)?" | sort -u | tr '\n' ' ')
[ "$LABEL" = "$REV12 " ] || refuse "the binary's label is [$LABEL], not exactly $REV12"
say "   binary   : $REL/pii  label=$REV12  $(sha256sum "$REL/pii" | cut -c1-16)…"
[ -s "$CARD" ] || refuse "no deploy card at $CARD"
"$REL/pii" deploy-card render --card "$CARD" >/dev/null 2>&1 || refuse "the deploy card does not render with all five fields"
grep -q "mode=fresh\|fresh-gate.*PASS" "$CARD" || refuse "the card's gate verdict does not quote a fresh-gate PASS"
grep -q "$REV7" "$CARD" || refuse "the card does not name revision $REV7"
say "   card     : $CARD (five fields, fresh-gate PASS quoted)"
sshx 'true' || refuse "cannot ssh to $HOST"
if [ -z "$REMOTE_ROOT" ]; then REMOTE_ROOT=$(sshx 'printf %s "$HOME"') || refuse "cannot read the host home"; fi
say "   host home: $REMOTE_ROOT   no-unit=$NO_UNIT"
FREE=$(sshx "df -BG '$REMOTE_ROOT' | tail -1 | tr -s ' ' | cut -d' ' -f4 | tr -d G"); [ "${FREE:-0}" -ge 5 ] || refuse "host has ${FREE:-?}G free"
LLAMA=$(sshx 'curl -s -o /dev/null -w %{http_code} --max-time 5 http://10.10.10.217:8082/v1/models'); [ "$LLAMA" = 200 ] || refuse "llama.cpp does not answer ($LLAMA)"
# the graphs that ship: the manifest plus the two the generated config names; each must lint with the NEW binary
mapfile -t SHIP < <(sed -e 's/#.*//' -e 's/[[:space:]]//g' "$REPO/graphs/minimal.txt" | grep -v '^$'); SHIP+=(discord-triage discord-digest)
EXP=$(git -C "$REPO" rev-parse --show-toplevel); TMPG=$(mktemp -d /projects/_tmp/deploy-graphs-XXXXXX); git -C "$REPO" archive "$REV12" graphs | tar -x -C "$TMPG"
python3 -I "$REPO/deploy/graph-closure.py" "$TMPG/graphs" "${SHIP[@]}" >/dev/null 2>&1 || refuse "the shipped graph set is not closed or not pinned (graph-closure.py)"
for g in "${SHIP[@]}"; do ( cd "$TMPG" && PII_RUNTIME_DIR="$TMPG" "$REL/pii" graph lint "$TMPG/graphs/$g.toml" >/dev/null 2>&1 ) || refuse "graph '$g' does not lint with the new binary"; done
say "   graphs   : ${#SHIP[@]} named, closed, pinned, all lint with the new binary"
if [ "$NO_UNIT" != 1 ]; then
  CUR=$(sshx 'PII_RUNTIME_DIR=/run/user/$(id -u)/pii ~/.local/bin/pii health --json 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin)[\"build\"][\"revision\"])"') || refuse "the running daemon does not answer health"
  say "   running  : $CUR  ->  $REV12"
  [ "$CUR" != "$REV12" ] || refuse "the host already runs $REV12"
fi
say "== 2. plan"
say "   stop pii.service · back up binary, config, graphs, 4 stores to $REMOTE_ROOT/pii-deploy/backup-<ts> · install binary"
say "   append [models.*] to ~/.local/pii/pii.toml if absent (spec 362) · sync ${#SHIP[@]} graphs (old bytes kept under .by-sha256)"
say "   start · health must say $REV12 · on failure: restore everything and start the old binary"
if [ "$APPLY" != 1 ]; then say "DRY RUN OK: nothing was changed. Re-run with --apply."; exit 0; fi

TS=$(date +%s)
say "== 3. apply ($TS)"
scp -q "$REL/pii" "$HOST:$REMOTE_ROOT/pii.upgrade-$TS" || { say "scp failed; nothing changed"; exit 1; }
( cd "$TMPG/graphs" && tar -cf - "${SHIP[@]/%/.toml}" fragments ) | sshx "mkdir -p '$REMOTE_ROOT/pii-deploy/stage-$TS' && tar -C '$REMOTE_ROOT/pii-deploy/stage-$TS' -xf -" || { say "graph upload failed; nothing changed"; exit 1; }
sshx "SHIPLIST='${SHIP[*]}' NO_UNIT=$NO_UNIT FORCE_BAD_HEALTH=${FORCE_BAD_HEALTH:-0} TS=$TS R='$REMOTE_ROOT' REV12=$REV12 bash -s" <<'REMOTE'
set -u
B=$R/pii-deploy/backup-$TS; S=$R/pii-deploy/stage-$TS
BIN=$R/.local/bin/pii; CONF=$R/.local/pii/pii.toml; GR=$R/.local/state/pii/graphs; ST=$R/.local/state/pii
RT=/run/user/$(id -u)/pii; [ "$NO_UNIT" = 1 ] && RT=$R/run/pii   # the daemon reads $RT/graphs (a real tmpfs dir), not only the state dir
ctl(){ [ "$NO_UNIT" = 1 ] && return 0; systemctl --user "$@"; }
health_rev(){ [ "${FORCE_BAD_HEALTH:-0}" = 1 ] && [ ! -f "$R/.rolled" ] && { echo "bad-revision"; return; }; [ "$NO_UNIT" = 1 ] && { echo "$REV12"; return; }; PII_RUNTIME_DIR=$RT $BIN health --json 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["build"]["revision"])' 2>/dev/null; }
rollback(){
  echo "ROLLBACK: $1"; touch "$R/.rolled"; ctl stop pii
  [ -f "$B/pii" ] && mv -f "$BIN" "$R/pii.failed-$TS" 2>/dev/null; [ -f "$B/pii" ] && cp -p "$B/pii" "$BIN.rb" && mv -f "$BIN.rb" "$BIN"
  [ -f "$B/pii.toml" ] && cp -p "$B/pii.toml" "$CONF"
  [ -d "$B/graphs" ] && { mkdir -p "$R/pii-deploy/failed-$TS"; mv "$GR" "$R/pii-deploy/failed-$TS/state-graphs" && mkdir -p "$GR" && cp -a "$B/graphs/." "$GR/"; }
  [ -d "$B/rtgraphs" ] && { mkdir -p "$R/pii-deploy/failed-$TS"; mv "$RT/graphs" "$R/pii-deploy/failed-$TS/rt-graphs" && mkdir -p "$RT/graphs" && cp -a "$B/rtgraphs/." "$RT/graphs/"; }
  for f in store memory approval heal queue; do [ -f "$B/$f.redb" ] && cp -p "$B/$f.redb" "$ST/$f.redb"; done
  ctl start pii; sleep 8; echo "ROLLBACK DONE: running revision $(health_rev)"; exit 1
}
mkdir -p "$B" || { echo "cannot make $B"; exit 1; }
echo "backup dir $B"
ctl stop pii; sleep 2
[ "$NO_UNIT" = 1 ] || { systemctl --user is-active --quiet pii && { echo "daemon still active after stop; aborting before any change"; ctl start pii; exit 1; }; }
cp -p "$BIN" "$B/pii" && cp -p "$CONF" "$B/pii.toml" && mkdir -p "$B/graphs" && cp -a "$GR/." "$B/graphs/" || { echo "backup of binary/config/graphs failed"; ctl start pii; exit 1; }
mkdir -p "$B/rtgraphs"; [ -d "$RT/graphs" ] && cp -a "$RT/graphs/." "$B/rtgraphs/"
for f in store memory approval heal queue; do [ -f "$ST/$f.redb" ] && { cp -p "$ST/$f.redb" "$B/$f.redb" || { echo "backup of $f.redb failed"; ctl start pii; exit 1; }; }; done
( cd "$B" && sha256sum pii pii.toml *.redb > MANIFEST.sha256 ) && echo "backed up: $(ls "$B" | tr '\n' ' ')"
# binary: new inode, so a running process is not disturbed; the daemon is stopped anyway
mv -f "$R/pii.upgrade-$TS" "$BIN.new" && chmod 0755 "$BIN.new" && mv -f "$BIN.new" "$BIN" || rollback "could not install the binary"
echo "installed: $($BIN --version 2>&1 | head -1)"
# [models] block, only if the config has none (spec 362: one place names the models)
if ! grep -q '^\[models\.' "$CONF"; then
  cat >> "$CONF" <<'TOML'

# --- appended by scripts/deploy-upgrade-host.sh (spec 362): the one place that names the models ---
[models.workhorse]
model = "deepseek-v4.1-flash"
provider = "ollama-cloud"

[models.planner]
model = "glm-5.3"
provider = "ollama-cloud"
TOML
  python3 -c 'import tomllib,sys; tomllib.load(open(sys.argv[1],"rb"))' "$CONF" || rollback "the config does not parse after the [models] block"
  echo "config: [models.workhorse] [models.planner] appended"
else echo "config: [models] already present, left as is"; fi
# graphs: old bytes retained by digest (a user graph's pin still resolves), then replaced
mkdir -p "$GR/.by-sha256"; n=0
for f in "$S"/*.toml; do
  g=$(basename "$f"); d="$GR/$g"
  if [ -e "$d" ] && ! cmp -s "$f" "$d"; then h=$(sha256sum "$d" | cut -d' ' -f1); [ -f "$GR/.by-sha256/$h.toml" ] || cp -p "$d" "$GR/.by-sha256/$h.toml"; fi
  cmp -s "$f" "$d" 2>/dev/null || { install -m 0644 "$f" "$d" && n=$((n+1)); }
done
echo "graphs: $n replaced or added in the state dir"
mkdir -p "$RT/graphs/fragments" "$GR/fragments"; m=0
for f in "$S"/*.toml; do cmp -s "$f" "$RT/graphs/$(basename "$f")" 2>/dev/null || { install -m 0644 "$f" "$RT/graphs/$(basename "$f")" && m=$((m+1)); }; done
[ -d "$S/fragments" ] && for f in "$S"/fragments/*.toml; do install -m 0644 "$f" "$GR/fragments/$(basename "$f")"; install -m 0644 "$f" "$RT/graphs/fragments/$(basename "$f")"; done
echo "graphs: $m placed in $RT/graphs, where the daemon reads them"
ctl start pii; sleep 10
REVNOW=$(health_rev); echo "health revision: ${REVNOW:-<no answer>}"
[ "$REVNOW" = "$REV12" ] || rollback "health does not name $REV12"
if [ "$NO_UNIT" != 1 ]; then
  ALT=$(echo $SHIPLIST | tr ' ' '|'); LIST=$(PII_RUNTIME_DIR=$RT $BIN graph list 2>&1)
  BAD=$(printf '%s\n' "$LIST" | grep -E "^($ALT) · .*unloadable"); LOADED=$(printf '%s\n' "$LIST" | grep -cE "^($ALT) · ")
  echo "graph list: $LOADED of $(echo $SHIPLIST | wc -w) shipped graphs listed by the daemon"
  [ -z "$BAD" ] || { printf '%s\n' "$BAD"; rollback "a shipped graph is unloadable in the started daemon"; }
  [ "$LOADED" -ge "$(echo $SHIPLIST | wc -w)" ] || rollback "the daemon lists fewer graphs than were shipped"
fi
[ "$NO_UNIT" = 1 ] || { [ "$(systemctl --user is-active pii)" = active ] || rollback "unit not active"; echo "unit: active, NRestarts=$(systemctl --user show pii -p NRestarts --value)"; }
echo "UPGRADE OK: $REVNOW (backup kept at $B)"
REMOTE
RC=$?
say "remote rc=$RC"; exit $RC
