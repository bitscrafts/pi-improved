#!/usr/bin/env bash
# run-batchC-pi.sh — dispatch one queue item to `pi`, on a non-Anthropic provider.
#
# Written 2026-10-19 on user directive: "dont implement it as the anthropic quota will exaust". The Claude worker
# (run-batchC-claude.sh) bills the subscription account; this one bills the ollama-cloud account instead, so the queue can
# keep working while the Claude window is protected.
#
# Contract is deliberately identical to run-batchC-claude.sh so cycle.sh can swap one for the other:
#   · prints MODEL, PROVIDER, SPEC, BRIEF, LOG, PAYLOAD before spending anything
#   · sets its own environment (HARD RULE 0d) and NEVER echoes the key
#   · resume-first: `pi --session <FILE>` on an existing session file, fresh otherwise
#   · records the session FILE so the next attempt continues rather than restarts
#
# Usage:  bash scripts/run-batchC-pi.sh <spec-number> [--resume] [brief-file]
# Env:    PI_MODEL (default glm-5.3-flash:cloud), PI_PROVIDER (default ollama-cloud)
set -uo pipefail
cd /projects/pi-improved || exit 1

# HARD STOP (user directive 2026-10-20, until further notice): the pi+glm lane is
# not used. Refuse before anything is spent, unless the operator names the override.
if [ "${PI_LANE_ENABLED:-0}" != "1" ]; then
  echo "REFUSED: the pi+glm implementation lane is disabled (user directive 2026-10-20: pi+glm not to be used until further notice)."
  echo "Use the claude lane (scripts/run-batchC-claude.sh). Override only with PI_LANE_ENABLED=1 and an explicit user instruction."
  exit 77
fi

N=${1:?usage: run-batchC-pi.sh <spec-number> [--resume] [brief-file]}
shift || true
RESUME=0
if [ "${1:-}" = "--resume" ]; then RESUME=1; shift; fi
BRIEF_FILE=${1:-}
MODEL=${PI_MODEL:-glm-5.3-flash:cloud}
PROVIDER=${PI_PROVIDER:-ollama-cloud}

SPEC=$(ls specs/"$N"-*.md 2>/dev/null | head -1)
[ -n "$SPEC" ] || { echo "ABORT: no spec specs/$N-*.md — nothing dispatched"; exit 2; }

# environment INSIDE the script: PATH plus the provider key, read from the 0600 env file, never printed
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
set -a; . /root/.config/pii/pii.env 2>/dev/null; set +a
[ -n "${OPENAI_API_KEY:-}" ] || { echo "ABORT: OPENAI_API_KEY absent from /root/.config/pii/pii.env"; exit 3; }

SDIR=_tmp/pi-batchC; mkdir -p "$SDIR"
SESS="$SDIR/session-$N.jsonl"
LOG="$SDIR/$N-$(date -u +%Y%m%dT%H%M%SZ).log"

BRIEF=${BRIEF_FILE:-$SDIR/brief-$N.md}
if [ ! -f "$BRIEF" ]; then
  printf '%s\n' "Implement $SPEC. Work only inside the repository. Do not commit. When done, run its Exit Criteria and paste the output." > "$BRIEF"
fi

echo "MODEL    : $MODEL"
echo "PROVIDER : $PROVIDER (ollama-cloud; the Claude account is NOT used by this dispatch)"
echo "SPEC     : $SPEC"
echo "BRIEF    : $BRIEF ($(wc -c < "$BRIEF") bytes)"
echo "SESSION  : $SESS $([ -f "$SESS" ] && echo '(exists -> resume)' || echo '(new)')"
echo "LOG      : $LOG"

ARGS=(--provider "$PROVIDER" --model "$MODEL" --api-key "$OPENAI_API_KEY")
if [ -f "$SESS" ] || [ "$RESUME" = 1 ]; then ARGS+=(--session "$SESS"); fi

pi -p "${ARGS[@]}" "$(cat "$BRIEF")" >"$LOG" 2>&1
RC=$?
echo "session-file $SESS" > "$SDIR/session-$N.txt"
echo "rc=$RC log=$LOG session=$SESS"
exit "$RC"
