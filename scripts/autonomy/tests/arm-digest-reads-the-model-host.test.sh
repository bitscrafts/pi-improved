#!/usr/bin/env bash
# arm-digest-reads-the-model-host-not-a-comment.test.sh — the arm script must take the model host from [llama]
# (2026-11-01: the first version took the first URL in the file, which was a comment pointing at a DIFFERENT model's
# server, and the installer wrote it into [llama] host; production's model would have stopped resolving).
set -u
cd "$(dirname "$0")/../../.." || exit 1
OK=0; FAILS=0
check(){ if [ "$2" = "$3" ]; then echo "ok $1"; OK=$((OK+1)); else echo "FAIL $1"; echo "  got '$2', want '$3'" >&2; FAILS=$((FAILS+1)); fi; }
T="$PWD/_tmp/arm-digest-host.$$"; trap 'find "$T" -depth -delete 2>/dev/null' EXIT
rm -rf "$T"; mkdir -p "$T"
cat > "$T/pii.toml" <<'CFG'
[core]
model = "llamacpp-server:granite-4.2-3b"
# loopback, http://127.0.0.1:8080/v1 -- an example in a comment, NOT the live host
[net]
allow = ["discord.com:443"]
[llama]
host = "http://10.10.10.217:8084/v1"
CFG
check "the host comes from [llama], not from a comment above it" \
  "$(CONFIG_DIR="$T" bash deploy/arm-digest.sh --print-host)" "http://10.10.10.217:8084/v1"
# CONTROL: a config whose [llama] section has no host must print nothing, so the caller refuses instead of guessing
printf '[llama]\nmodel = "x"\n' > "$T/pii.toml"
check "CONTROL no [llama] host prints nothing" "$(CONFIG_DIR="$T" bash deploy/arm-digest.sh --print-host)" ""
# and a second [llama]-like section name must not be mistaken for it
printf '[llama-extra]\nhost = "http://evil:1/v1"\n[llama]\nhost = "http://good:8084/v1"\n' > "$T/pii.toml"
check "a similarly-named section is not mistaken for [llama]" \
  "$(CONFIG_DIR="$T" bash deploy/arm-digest.sh --print-host)" "http://good:8084/v1"

echo "$OK ok, $FAILS FAIL"
[ "$FAILS" -eq 0 ]
