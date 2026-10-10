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
[ -n "$N" ] || { echo "usage: run-batchC-claude.sh <spec-number> [--resume <session-id>] [extra-brief-file]"; exit 2; }
SPEC=$(ls "$REPO"/specs/$N-*.md 2>/dev/null | head -1) || { echo "no spec $N"; exit 2; }
[ -f "$SPEC" ] || { echo "no spec $N"; exit 2; }
TAG=$(basename "$SPEC" .md)
RESUME_SID=""; EXTRA_BRIEF=""
shift || true
while [ $# -gt 0 ]; do
  case "$1" in
    --resume) RESUME_SID="${2:?--resume needs a session id}"; shift 2 ;;
    *) EXTRA_BRIEF="$1"; shift ;;
  esac
done
# dirty-tree guard: the worker must start on the committed baseline.
# ALLOW_DIRTY=1 is the explicit, logged override for resuming a spec whose WIP is the dirty
# change itself (e.g. a run cut off mid-edit) — never for starting unrelated work.
if [ -n "$(git status --porcelain)" ]; then
  if [ -n "${ALLOW_DIRTY:-}" ]; then
    echo "== ALLOW_DIRTY: continuing with WIP in the tree:"; git status --porcelain
  else
    echo "STOPPED: dirty tree (set ALLOW_DIRTY=1 only to resume this spec's own WIP):"; git status --porcelain; exit 8
  fi
fi
LOG="$LOGD/$TAG.$(date +%s).log"
BRIEF="$LOGD/brief-$TAG.md"
cat > "$BRIEF" <<EOF
# Implement spec $TAG (follow the spec file as the CONTRACT)
Read $SPEC and execute it exactly: TDD (RED observed before GREEN), minimal diffs.
Full gate context: fmt/clippy(-D warnings)/workspace tests; quality-gate.sh is the arbiter.
HARD RULES: do NOT create/edit/rewrite anything under specs/ (read-only for implementers);
do NOT commit; do NOT run git commit/push; leave the tree for the orchestrator.
DESTRUCTIVE COMMANDS ARE BANNED, absolutely: never run rm -rf (or rm on a path you did not create in
this run), never git checkout/restore ., git reset --hard, or a bare git stash in this shared tree,
and never touch /dev, /proc, /sys or /root. Scratch YOU create under /projects/_tmp/ may be removed
with your own guarded script; anything pre-existing (including _tmp items you did not create) is left
alone or moved aside with mv. The only bulk reclaim allowed is cargo clean. A stray rm that "misses"
is still a breach and must be reported.
Scratch exports under /projects/_tmp/ only. Work ENTIRELY in foreground tool calls; no
background processes, no /loop, no scheduling. If budget runs out mid-work, write a
COMPLETE handoff envelope (done/remaining/next-start) to $LOGD/HANDOFF-$TAG.md and stop
honestly — never compress the work to fake completion. Final line: SPEC-DONE $N criteria=<p>/<t> with every criterion command + exit code listed.
EOF
[ -n "$EXTRA_BRIEF" ] && { printf '\n\n' >> "$BRIEF"; cat "$EXTRA_BRIEF" >> "$BRIEF"; }
RESUME_ARGS=()
if [ -n "$RESUME_SID" ]; then RESUME_ARGS=(--resume "$RESUME_SID"); echo "== RESUMING session $RESUME_SID"; fi
stdbuf -oL -eL claude "${RESUME_ARGS[@]}" -p "$(cat "$BRIEF")" \
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