#!/usr/bin/env bash
# devbox-install-binary.test.sh — the devbox refresh installs the binary the build wrote (spec 279).
# Runs modules/devbox/install-binary.sh on scratch files and reads daemon.sh's call sites from text; never runs daemon.sh, ssh or podman.
# Exits 0 only if every assertion holds. Scratch lives under _tmp/ and is removed by this script's own trap.
set -u
cd "$(dirname "$0")/../../.." || exit 1
INSTALL=modules/devbox/install-binary.sh
DAEMON=modules/devbox/daemon.sh
LIT=/projects/pi-improved/target/debug/pii
OK=0
FAILS=0
# check <name> <got> <want>: an exact comparison, one ok or FAIL line per assertion
check(){
  if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi
}
# sha12 <file>: the first 12 hex digits of its sha256
sha12(){ sha256sum "$1" | cut -c1-12; }
# calls <file>: the non-comment lines that run install-binary.sh, as <lineno>:<text> (a comment's first non-blank character is #)
calls(){ grep -nF 'install-binary.sh' "$1" | grep -vE '^[0-9]+:[[:blank:]]*#'; }
# target_text <line>: the target-dir argument as the call line spells it, between "install-binary.sh " and " $PII"
target_text(){ local t=${1#*install-binary.sh }; printf '%s' "${t%% \$PII*}"; }
# expand <text> <env...>: the two shells the call passes through: the host reads the text inside double quotes, then the container's sh reads the bare word
expand(){
  local text=$1 host
  shift
  host=$(unset CARGO_TARGET_DIR; eval "printf '%s' \"$text\"")
  env -i PATH="$PATH" "$@" sh -c "printf '%s' $host"
}
# expansions <file> <env...>: the container's reading of the target-dir argument at every live call site, space-separated
expansions(){
  local f=$1 l out=
  shift
  while IFS= read -r l; do out="$out $(expand "$(target_text "${l#*:}")" "$@")"; done <<<"$(calls "$f")"
  out=${out# }
  echo "$out"
}
# wired <file>: wired only when there are exactly two live call sites, none names the literal path, and the target-dir argument reads as the container's env says at every site
wired(){
  local w
  w=$(calls "$1")
  [ "$(echo "$w" | grep -c .)" = 2 ] || { echo unwired; return; }
  [ "$(echo "$w" | grep -cF "$LIT")" = 0 ] || { echo unwired; return; }
  [ "$(expansions "$1" CARGO_TARGET_DIR=/planted)" = "/planted /planted" ] && [ "$(expansions "$1")" = "target target" ] && echo wired || echo unwired
}

T=_tmp/devbox-install-binary.$$
mkdir -p "$T"
trap 'find "$T" -depth -delete 2>/dev/null' EXIT

# (a) THE BUG: the install copies <target_dir>/debug/pii, never a decoy beside it
A=$T/a
mkdir -p "$A/root/target/debug" "$A/T/debug"
printf 'OLD' > "$A/root/target/debug/pii"; chmod +x "$A/root/target/debug/pii"
printf 'NEW' > "$A/T/debug/pii"; chmod +x "$A/T/debug/pii"
printf 'stale' > "$A/dest"
OUT=$(bash "$INSTALL" "$A/T" "$A/dest" 2>&1); RC=$?
check "(a) THE BUG: dest gets the bytes of T/debug/pii, not the decoy" "$(cat "$A/dest")" "NEW"
check "(a) the install prints installed with the digest of NEW" "$OUT" "installed $(sha12 "$A/T/debug/pii") from $A/T/debug/pii"
check "(a) the good install exits 0" "$RC" "0"

# (b) CONTROL: a missing or non-executable source exits 1, names the exact path, and touches nothing
B=$T/b
mkdir -p "$B/T"
printf 'KEEP' > "$B/dest"
OUT=$(bash "$INSTALL" "$B/T" "$B/dest" 2>&1); RC=$?
check "(b) a missing source exits 1" "$RC" "1"
check "(b) the failure names the exact missing path" "$(grep -qF "$B/T/debug/pii" <<<"$OUT" && echo yes || echo no)" "yes"
check "(b) the existing dest is byte-identical" "$(cat "$B/dest")" "KEEP"
check "(b) no dest.new is created" "$(test -e "$B/dest.new" && echo yes || echo no)" "no"
mkdir -p "$B/T/debug"
printf 'NOTEXEC' > "$B/T/debug/pii"
OUT=$(bash "$INSTALL" "$B/T" "$B/dest" 2>&1); RC=$?
check "(b) a source that is not executable exits 1" "$RC" "1"
check "(b) a non-executable source leaves dest byte-identical" "$(cat "$B/dest")" "KEEP"

# (c) CONTROL: a cp that truncates its copy is caught by the digest, dest.new is removed, dest is unchanged
C=$T/c
mkdir -p "$C/T/debug" "$C/shim"
printf 'NEWBIN-full-content' > "$C/T/debug/pii"; chmod +x "$C/T/debug/pii"
printf 'KEEP' > "$C/dest"
printf '#!/bin/sh\nhead -c 3 "$1" > "$2"\n' > "$C/shim/cp"; chmod +x "$C/shim/cp"
OUT=$(PATH="$C/shim:$PATH" bash "$INSTALL" "$C/T" "$C/dest" 2>&1); RC=$?
check "(c) a truncating cp makes the install exit 1" "$RC" "1"
check "(c) the mismatch removes dest.new" "$(test -e "$C/dest.new" && echo yes || echo no)" "no"
check "(c) dest is unchanged after the mismatch" "$(cat "$C/dest")" "KEEP"

# (d) CONTROL: wrong argument counts exit 2 with a usage line
bash "$INSTALL" >/dev/null 2>&1; R0=$?
bash "$INSTALL" "$T/a" >/dev/null 2>&1; R1=$?
bash "$INSTALL" "$T/a" "$T/a/dest" "$T/x" >/dev/null 2>&1; R3=$?
check "(d) no arguments exit 2" "$R0" "2"
check "(d) one argument exits 2" "$R1" "2"
check "(d) three arguments exit 2" "$R3" "2"
check "(d) the usage line is printed" "$(bash "$INSTALL" 2>&1 | grep -c usage)" "1"

# (e) CONTROL: a good install over an existing different dest replaces it and leaves no .new
E=$T/e
mkdir -p "$E/T/debug"
printf 'NEW2' > "$E/T/debug/pii"; chmod +x "$E/T/debug/pii"
printf 'OLDER' > "$E/dest"
bash "$INSTALL" "$E/T" "$E/dest" >/dev/null 2>&1; RC=$?
check "(e) a good install over a different dest exits 0" "$RC" "0"
check "(e) the different dest is replaced" "$(cat "$E/dest")" "NEW2"
check "(e) no dest.new is left behind" "$(test -e "$E/dest.new" && echo yes || echo no)" "no"

# (f) THE WIRING, read from the live daemon.sh text
check "(f) exactly two live call sites of install-binary.sh" "$(calls "$DAEMON" | grep -c .)" "2"
check "(f) no live call site names the literal target path" "$(calls "$DAEMON" | grep -cF "$LIT")" "0"
check "(f) with CARGO_TARGET_DIR=/planted every site reads /planted" "$(expansions "$DAEMON" CARGO_TARGET_DIR=/planted)" "/planted /planted"
check "(f) with CARGO_TARGET_DIR unset every site reads target" "$(expansions "$DAEMON")" "target target"
check "(f) the live daemon.sh is wired" "$(wired "$DAEMON")" "wired"
check "bash -n install-binary.sh" "$(bash -n "$INSTALL" && echo ok)" "ok"
check "bash -n daemon.sh" "$(bash -n "$DAEMON" && echo ok)" "ok"

# (g) CONTROL: a scratch daemon.sh with the literal path restored in the call site reddens the wiring check
G=$T/g-daemon.sh
sed 's#/projects/pi-improved/modules/devbox/install-binary.sh#/projects/pi-improved/target/debug/pii#' "$DAEMON" > "$G"
check "(g) CONTROL: a daemon.sh with the literal path restored is unwired" "$(wired "$G")" "unwired"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
