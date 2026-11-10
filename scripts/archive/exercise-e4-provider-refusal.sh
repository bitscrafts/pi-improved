#!/usr/bin/env bash
# exercise-e4-provider-refusal.sh — E4: the model provider is unreachable; pii refuses NAMING it, never hangs.
# Inject: the llama.cpp base URL is pointed at a dead port (the E4 fault of the matrix: "stop the
# llama.cpp server", done here without touching the real one). The CLI run must terminate on its own
# (timeout never fires), with a refusal naming the provider/host — not an empty hang.
#
# Measured 2026-10-19 (harness fix): a bare `pii -p` prefers the LIVE daemon's UDS socket
# (cmd_oneshot.rs:143-152, F5.6 — the daemon is never auto-started but wins when present), so the
# scratch PII_CONFIG was never read and the fault never fired: the morning's "PASS" ran against the
# daemon's own provider. The engine under test is reached with --connect loopback, and the CONTROL
# arm (same command, the operator's real config, whose llama host answers) proves the injection:
# control succeeds, injected run refuses.
# Run: bash scripts/archive/exercise-e4-provider-refusal.sh <model-spec>   (default llamacpp-server:granite-4.2-3b)
set -uo pipefail
cd /projects/pi-improved || exit 1
export XDG_RUNTIME_DIR=/run/user/0
PII=/root/.local/bin/pii
MODEL="${1:-llamacpp-server:granite-4.2-3b}"
T=$(mktemp -d /projects/_tmp/ex19e4-XXXXXX)
# the CLI reads [llama] host from the config file (PII_CONFIG), not env: a scratch
# config points the llama base at a dead LOOPBACK port (connection refused in ms;
# never a black-hole IP, and the real server is untouched).
cat > "$T/pii.toml" <<CFG
[core]
model = "llamacpp-server:granite-4.2-3b"

[llama]
host = "http://127.0.0.1:9/v1"
CFG
export PII_CONFIG="$T/pii.toml"
echo "model=$MODEL  llama_base=http://127.0.0.1:9/v1 (via $PII_CONFIG, --connect loopback)"
START=$(date +%s)
timeout 90 "$PII" -p "Reply with exactly: PONG" --model "$MODEL" --connect loopback > "$T/out.jsonl" 2>&1
RC=$?
END=$(date +%s)
echo "rc=$RC elapsed=$((END - START))s (a hang would be rc=124 at 90s)"
echo "--- injected output ---"
cut -c1-240 "$T/out.jsonl"
NAME_OK=$(grep -ci "no address reachable\|refused\|unavailable\|provider" "$T/out.jsonl")

# CONTROL: the same command over the operator's real config (its llama host answers).
# If the control cannot succeed, the host itself is down and neither arm proves anything.
unset PII_CONFIG
set -a; . /root/.config/pii/pii.env 2>/dev/null; set +a
timeout 90 "$PII" -p "Reply with exactly: PONG" --model "$MODEL" --connect loopback > "$T/control.jsonl" 2>&1
C_RC=$?
CTRL_OK=$([ "$C_RC" -eq 0 ] && grep -c "PONG" "$T/control.jsonl" || echo 0)
echo "--- control (real llama host): rc=$C_RC PONG lines=$CTRL_OK ---"

if [ "$RC" -ne 124 ] && [ "$NAME_OK" -ge 1 ] && [ "$CTRL_OK" -ge 1 ]; then
  echo "E4 PASS — terminated on its own, named the refusal, and the control proves the dial:"
  grep -m1 "unavailable\|refused" "$T/out.jsonl" | cut -c1-220
elif [ "$CTRL_OK" -eq 0 ]; then
  echo "E4 INCONCLUSIVE — the control arm failed (rc=$C_RC); the llama host itself must answer for this exercise to mean anything"
  exit 3
else
  echo "E4 FAIL — rc=$RC naming_hits=$NAME_OK (need: no hang + a named refusal)"
  exit 1
fi