#!/usr/bin/env bash
# Spec 142: undo deploy/install-pii.sh. Stops the unit if running, then MOVES the newest
# dated "${PREFIX}.old-<ts>" contents back (a move, not a copy). Binaries it replaces are moved
# to "${PREFIX}.rolledback-<ts>", never deleted. Same PREFIX/SCOPE/CONFIG_DIR/UNIT_DIR variables
# as install-pii.sh. SYSTEMCTL overrides the systemctl command (tests).
set -euo pipefail

SCOPE="${SCOPE:-user}"
PREFIX="${PREFIX:-$HOME/.local/bin}"
case "$SCOPE" in
  user)   CONFIG_DIR="${CONFIG_DIR:-$HOME/.config/pii}"; UNIT_DIR="${UNIT_DIR:-$HOME/.config/systemd/user}"
          CTL="${SYSTEMCTL:-systemctl} --user" ;;
  system) CONFIG_DIR="${CONFIG_DIR:-/etc/pii}"; UNIT_DIR="${UNIT_DIR:-/etc/systemd/system}"
          CTL="${SYSTEMCTL:-systemctl}" ;;
  *) echo "ABORT: SCOPE must be user or system" >&2; exit 2 ;;
esac
STATE_DIR="${STATE_DIR:-$([ "$SCOPE" = system ] && echo /var/lib/pii || echo "$HOME/.local/state/pii")}"
TS="$(date +%s)"

ASIDE=""
for d in "${PREFIX}".old-*; do
  [ -d "$d" ] && ASIDE="$d"   # glob is sorted: the last is the newest timestamp
done
[ -n "$ASIDE" ] || { echo "ABORT: no ${PREFIX}.old-* directory to restore from" >&2; exit 1; }

shopt -s nullglob
held=("$ASIDE"/*)
shopt -u nullglob
[ "${#held[@]}" -gt 0 ] || { echo "NOTHING TO RESTORE: $ASIDE holds no files" >&2; exit 1; }

if $CTL is-active --quiet pii 2>/dev/null; then
  $CTL stop pii
  echo "STOPPED: pii unit"
fi

REPLACED="${PREFIX}.rolledback-${TS}"
restored=0
for f in "$ASIDE"/*; do
  [ -e "$f" ] || continue
  name="$(basename "$f")"
  case "$name" in
    pii.toml)    dest="$CONFIG_DIR/pii.toml" ;;
    pii.service) dest="$UNIT_DIR/pii.service" ;;
    graph.*.toml) dest="$STATE_DIR/graphs/${name#graph.}" ;;
    *)           dest="$PREFIX/$name" ;;
  esac
  if [ -e "$dest" ]; then
    mkdir -p "$REPLACED"
    mv "$dest" "$REPLACED/$name"
  fi
  mv "$f" "$dest"
  echo "RESTORED: $dest  (from $ASIDE)"
  restored=$((restored + 1))
done
echo "ROLLBACK DONE: $restored file(s) restored from $ASIDE; replaced files kept in $REPLACED"
[ "$restored" -gt 0 ] && echo "NEXT: $CTL daemon-reload && $CTL restart pii   (if the unit is enabled)"
exit 0
