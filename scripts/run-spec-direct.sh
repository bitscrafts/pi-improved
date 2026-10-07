#!/usr/bin/env bash
# run-spec-direct.sh — the pii IMPLEMENTATION lane: pi called DIRECTLY (user rule 2026-08-19,
# verbatim: "No pi-orchestration!"), glm-5.3-flash:cloud, dispatched from a committed script.
# usage: run-spec-direct.sh <spec-number> [spec-number...]   e.g. 73 74
# Contract: resume-first (session file per spec-tag lives under _tmp/pi-sessions-direct/),
# worker never commits, orchestrator verifies after; NO pi-workhorse/harness for this project.
set -uo pipefail
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
export PATH="/usr/local/cargo/bin:/root/.local/bin:$PATH"
export IS_SANDBOX=1
set -a; [ -f /root/.hermes/.env ] && . /root/.hermes/.env; set +a
# pin the worker lane INSIDE the script (never trust the caller's env)
export PI_PROVIDER=ollama-cloud
export PI_MODEL=glm-5.3-flash:cloud
unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN ANTHROPIC_OAUTH_TOKEN CLAUDE_CODE_OAUTH_TOKEN 2>/dev/null || true

[ $# -ge 1 ] || { echo "usage: $0 <spec-number>..." >&2; exit 2; }
SESSDIR=/projects/pi-improved/_tmp/pi-sessions-direct
mkdir -p "$SESSDIR"

overall=0
for N in "$@"; do
  SPEC=$(ls /projects/pi-improved/specs/${N}-*.md 2>/dev/null | head -1)
  [ -n "$SPEC" ] || { echo "!! spec $N not found" >&2; overall=2; continue; }
  TAG=$(basename "$SPEC" .md)
  echo "== direct-pi lane: $TAG  model=$PI_MODEL"
  cd /projects/pi-improved

  # resume-first: continue this spec's session if one exists (--session takes a FILE;
  # --session-dir DIR is storage/lookup only — passing it with a "resume" intent silently
  # starts a NEW session. The check: the old file GREW or a new file appeared beside it.)
  SESSDIR_TAG="$SESSDIR/$TAG"
  existing=$(ls -t "$SESSDIR_TAG"/*.jsonl 2>/dev/null | head -1)
  RESUME_ARGS=()
  if [ -n "${existing:-}" ] && [ -f "$existing" ]; then
    echo "   resuming session file: $existing"
    RESUME_ARGS=(--session "$existing")
  else
    mkdir -p "$SESSDIR_TAG"
    RESUME_ARGS=(--session-dir "$SESSDIR_TAG")
    echo "   fresh session (storage dir): $SESSDIR_TAG"
  fi

  timeout 3300 pi --provider "$PI_PROVIDER" --model "$PI_MODEL" --thinking off \
    "${RESUME_ARGS[@]}" -p "RESUME. Continue the previous session EXACTLY where the handoff envelope says: read /projects/_tmp/spec73/HANDOFF-73.md (spec 73) or /projects/_tmp/spec-74-incomplete-handoff-2026-10-07.md (spec 74) FIRST — recon is done, designs are settled, do NOT re-derive. Execute 'What remains (in order)' verbatim: RED (observe it fail on the scratch export), GREEN minimal, run every Exit Criteria command verbatim, and print as the final line: SPEC-DONE <N> criteria=<passed>/<total> listing each criterion command + its exit code. Do NOT commit; do not edit specs/. Scratch under /projects/_tmp/ only."
  rc=$?
  echo "== direct-pi $TAG exit rc=$rc"
  [ "$rc" -ne 0 ] && overall=$rc
done
echo "BATCH-OVERALL=$overall"
exit "$overall"