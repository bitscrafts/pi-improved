#!/usr/bin/env bash
# run-batchC-claude.sh — pii batch C on the CLAUDE+SONNET lane (user directive 2026-10-09:
# "whats next in pii? proceed with claude+sonnet!").
# One spec per claude -p run; specs are READ-ONLY for the worker; workers never commit.
# Stream-json ALWAYS (live monitoring); session_id captured to file for resume.
# Env is set INSIDE the script (HARD RULE 0d: background shells inherit nothing).
set -euo pipefail
export PATH="/usr/local/cargo/bin:/root/.local/bin:$PATH"
export HOME=/root
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
REPO=/projects/pi-improved
LOGD=$REPO/_tmp/claude-batchC
mkdir -p "$LOGD"
cd "$REPO"
N="${1:-}"
[ -n "$N" ] || { echo "usage: run-batchC-claude.sh <spec-number>"; exit 2; }
SPEC=$(ls "$REPO"/specs/$N-*.md 2>/dev/null | head -1) || { echo "no spec $N"; exit 2; }
[ -f "$SPEC" ] || { echo "no spec $N"; exit 2; }
TAG=$(basename "$SPEC" .md)
# dirty-tree guard: the worker must start on the committed baseline
if [ -n "$(git status --porcelain)" ]; then echo "STOPPED: dirty tree:"; git status --porcelain; exit 8; fi
LOG="$LOGD/$TAG.$(date +%s).log"
BRIEF="$LOGD/brief-$TAG.md"
cat > "$BRIEF" <<EOF
# Implement spec $TAG (follow the spec file as the CONTRACT)
Read $SPEC and execute it exactly: TDD (RED observed before GREEN), minimal diffs.
Full gate context: fmt/clippy(-D warnings)/workspace tests; quality-gate.sh is the arbiter.
HARD RULES: do NOT create/edit/rewrite anything under specs/ (read-only for implementers);
do NOT commit; do NOT run git commit/push; leave the tree for the orchestrator.
Scratch exports under /projects/_tmp/ only. Work ENTIRELY in foreground tool calls; no
background processes, no /loop, no scheduling. If budget runs out mid-work, write a
COMPLETE handoff envelope (done/remaining/next-start) to $LOGD/HANDOFF-$TAG.md and stop
honestly — never compress the work to fake completion. Final line: SPEC-DONE $N criteria=<p>/<t> with every criterion command + exit code listed.
EOF
stdbuf -oL -eL claude -p "$(cat "$BRIEF")" \
  --permission-mode bypassPermissions --model sonnet \
  --output-format stream-json --verbose --include-partial-messages \
  --disallowedTools 'Edit(specs/**) Write(specs/**)' \
  >"$LOG" 2>"$LOG.err"
RC=$?
python3 - "$LOG" <<'PY' > "$LOGD/session-$TAG.txt"
import json,sys
last=None
for line in open(sys.argv[1]):
    try: d=json.loads(line)
    except Exception: continue
    if d.get('type')=='result': last=d
if last: print(last.get('session_id','NO-SESSION'), last.get('num_turns'), last.get('total_cost_usd'), last.get('stop_reason'))
PY
echo "== direct-claude $TAG exit rc=$RC (session/turns/cost/stop_reason: $(cat "$LOGD/session-$TAG.txt" 2>/dev/null))"
echo "== tree after: $(git status --porcelain | wc -l) dirty paths"
git status --porcelain | head -12
exit $RC