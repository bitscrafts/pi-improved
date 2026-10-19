#!/usr/bin/env bash
# exercise-e4-provider-refusal.sh — E4: the model provider is unreachable; pii refuses NAMING it, never hangs.
# Inject: the llama.cpp base URL is pointed at a dead port (the E4 fault of the matrix: "stop the
# llama.cpp server", done here without touching the real one). The CLI run must terminate on its own
# (timeout never fires), with a refusal naming the provider/host — not an empty hang.
# Run: bash scripts/exercise-e4-provider-refusal.sh <model-spec>   (default llamacpp-server:granite-4.2-3b)
set -uo pipefail
cd /projects/pi-improved || exit 1
export XDG_RUNTIME_DIR=/run/user/0
PII=/root/.local/bin/pii
MODEL="${1:-llamacpp-server:granite-4.2-3b}"
# the CLI reads [llama] host from the config file (PII_CONFIG), not env: a scratch
# config points the llama base at a dead LOOPBACK port (connection refused in ms;
# never a black-hole IP, and the real server is untouched).
T=$(mktemp -d /projects/_tmp/ex19e4-XXXXXX)
cat > "$T/pii.toml" <<CFG
[core]
model = "openai:glm-5.3-flash"

[llama]
host = "http://127.0.0.1:9/v1"
CFG
export PII_CONFIG="$T/pii.toml"
echo "model=$MODEL  llama_base=http://127.0.0.1:9/v1 (via $PII_CONFIG)"
START=$(date +%s)
timeout 90 "$PII" -p "Reply with exactly: PONG" --model "$MODEL" > "$T/out.jsonl" 2>&1
RC=$?
END=$(date +%s)
echo "rc=$RC elapsed=$((END - START))s (a hang would be rc=124 at 90s)"
echo "--- output ---"
cut -c1-240 "$T/out.jsonl"
NAME_OK=$(grep -ci "no address reachable\|refused\|unavailable\|provider" "$T/out.jsonl")
if [ "$RC" -ne 124 ] && [ "$NAME_OK" -ge 1 ]; then
  echo "E4 PASS — terminated on its own and named the refusal:"
  grep -m1 "unavailable\|refused" "$T/out.jsonl" | cut -c1-220
else
  echo "E4 FAIL — rc=$RC naming_hits=$NAME_OK (need: no hang + a named refusal)"
fi