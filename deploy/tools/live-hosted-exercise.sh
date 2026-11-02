#!/usr/bin/env bash
# live-hosted-exercise.sh — close 071 gap 3: run pii's OWN net + model stack against a real hosted provider.
#
# Why this exists: the stream-truncation defect (spec 191) is fixed at a site in pii-net, and the code is proven by
# tests — but the PATH has not been exercised since 2026-10-19, because the daemon runs on a local plain-HTTP lane
# and pii pins webpki roots (no CA override), so a local truncating peer cannot be trusted. A hosted provider is the
# way to exercise it honestly. The operator authorised the ollama-cloud credentials for exactly this.
#
# What it does: reads the key in-process (never printed, never an argument), lists the models the account serves,
# picks one, and runs `crates/pii-model/tests/live_smoke.rs` — the test that uses the real `pii-net` transport with an
# open egress grant. Then runs `stream_close_is_eof` for the truncation semantics.
set -uo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
AUTH=/root/.hermes/auth.json
BASE=${BASE:-https://ollama.com/v1}

# The provider registry (/root/.hermes/auth.json) names the SECRET'S SOURCE, not the secret: for ollama-cloud it is
# `env:OLLAMA_API_KEY`. That variable is set on the container's init (pid 1) and inherited by the agent, not by an
# interactive shell — so it is read from /proc/1/environ. It is never printed and never passed as an argument.
key=$(tr '\0' '\n' < /proc/1/environ 2>/dev/null | sed -n 's/^OLLAMA_API_KEY=//p' | head -1)
if [ -z "${key:-}" ]; then
  echo "REFUSED: no ollama-cloud key found in $AUTH (label does not mention ollama)" >&2
  exit 2
fi
echo "credential: found in $AUTH (value never printed)"

models=$(OPENAI_API_KEY="$key" BASE="$BASE" python3 - <<'PY'
import json, os, urllib.request
req = urllib.request.Request(os.environ["BASE"].rstrip("/") + "/models")
req.add_header("Authorization", "Bearer " + os.environ["OPENAI_API_KEY"])
req.add_header("User-Agent", "DiscordBot (https://github.com/pii, 0.1)")
try:
    with urllib.request.urlopen(req, timeout=25) as r:
        d = json.load(r)
    print(" ".join(m.get("id", "") for m in d.get("data", []))[:400])
except Exception as e:
    print(f"MODELS_ERR {e}")
PY
)
echo "models: ${models:0:300}"
case "$models" in
  MODELS_ERR*) echo "the account did not answer a model list: $models"; exit 3 ;;
esac

PICK=${PICK:-$(printf '%s' "$models" | tr ' ' '\n' | grep -viE 'embed|vision' | head -1)}
[ -n "$PICK" ] || { echo "REFUSED: no usable model in the list" >&2; exit 4; }
echo "model: $PICK"

cd "$REPO" || exit 1
. scripts/autonomy/env.sh 2>/dev/null || true
echo "=== live smoke against $BASE (real pii-net transport) ==="
OPENAI_API_KEY="$key" OPENAI_BASE_URL="$BASE" PII_LIVE_OPENAI_MODEL="$PICK" \
  timeout 300 cargo test -q -p pii-model --test live_smoke -- --ignored openai 2>&1 | tail -6
rc=$?
echo "=== the truncation semantics (unit, no network) ==="
timeout 300 cargo test -q -p pii-model --test stream_close_is_eof 2>&1 | tail -3
echo "live-hosted-exercise rc=$rc"
exit "$rc"
