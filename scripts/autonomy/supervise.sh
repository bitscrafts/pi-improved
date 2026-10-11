#!/usr/bin/env bash
# supervise.sh — the best-effort judgement layer, on the CLAUDE account tier. No Google/Gemini anywhere:
# the loop's ticks run no model at all, the workers are `claude`, and so is this.
#
# The user's constraint (2026-10-11, verbatim): "no google api usage". Measured cause: the first version of
# the supervisor was a Hermes cron AGENT, and that agent's model fell from gemini-3.8-flash to an nvidia
# fallback and then timed out at 90 s — a provider dependency on the critical path, in the wrong account.
#
# It does nothing when there is nothing to do: paused, a cycle in flight, or a closed window all mean silence.
set -uo pipefail
cd /projects/pi-improved || { echo "pii autonomy: cannot cd to the project"; exit 1; }
ST=_tmp/autonomy; LOGD=$ST/logs; mkdir -p "$LOGD"

grep -q PAUSED "$ST/control" 2>/dev/null && { echo "pii autonomy · paused — supervisor idle"; exit 0; }
flock -n "$ST/lock" true 2>/dev/null || { echo "pii autonomy · a cycle is in flight — supervisor idle"; exit 0; }
if ! bash scripts/autonomy/probe.sh >/dev/null 2>&1; then
  echo "pii autonomy · the Anthropic window is closed — supervisor idle, nothing spent"; exit 0
fi

TS=$(date -u +%Y%m%dT%H%M%SZ); LOG=$LOGD/supervise-$TS.log; BRIEF=$LOGD/supervise-brief-$TS.md
cat >"$BRIEF" <<'EOF'
You are the supervisor of an unattended loop in /projects/pi-improved. The deterministic loop
(scripts/autonomy/tick.sh -> cycle.sh, no model involved) does the dispatching, verifying and committing.
Your job is the judgement it cannot do. Work ENTIRELY in foreground tool calls; do NOT start background
processes, do NOT use any scheduling feature, do NOT restart the hermes gateway (it kills a running worker),
never delete anything (reclaim disk only with `cargo clean`), and never name an AI tool in a commit.

Read first: scripts/autonomy/plan.tsv, `python3 scripts/autonomy/state.py list`, the newest cycle log in
_tmp/autonomy/logs/, and the specs of any item in state needs-review/parked.

Then do ONE of these, in this order, and stop:
1. A cycle is in flight (lock held) -> say so and stop.
2. An item is `needs-review` or `parked` -> read why from the log. If the worker erred, resume its session
   (`ALLOW_DIRTY=1 bash scripts/run-batchC-claude.sh <spec-number> --resume <session> <corrective-brief>`).
   If the SPEC is wrong, fix the spec (specs/ are read-only for implementers, yours to author), commit it
   alone, and let the next tick dispatch it.
3. Otherwise: independently verify the last committed item — run its Exit Criteria yourself, and for a defect
   fix redden it with your own mutation and restore the file byte-identical (`sha256sum` before and after).
   Write what you found into that spec's `## Closure` section and commit that alone.
4. If every remaining item is parked or the queue is empty, author the next spec from the ranked list in
   _docs/06-research/059-opus-vision-judgement-2026-10-11.md section 4 (skipping what it says NOT to do), add
   its row to scripts/autonomy/plan.tsv, and commit them together.

Rules that are never negotiable: never weaken a test or an assertion to make an item pass; a worker's report
is a self-report, so check the tree and the criteria yourself; if the window is closed, print the reset text
and stop — waiting costs nothing, retrying blindly costs a window. Commits carry
`-c user.name="Marcelo Correa" -c user.email="mvcorrea@gmail.com"` and explicit paths only.

Your final answer is one short paragraph: what you did, what you found, and what the next tick should do.
EOF

stdbuf -oL -eL claude -p "$(cat "$BRIEF")" --allowedTools 'Read,Grep,Glob,Bash,Edit,Write' \
  --permission-mode bypassPermissions --max-turns 60 >"$LOG" 2>&1
RC=$?
SID=$(python3 - "$LOG" <<'PY' 2>/dev/null || true
import json,sys
last=None
for line in open(sys.argv[1],errors='replace'):
    try: d=json.loads(line)
    except Exception: continue
    if d.get('type')=='result': last=d
print((last or {}).get('session_id',''))
PY
)
echo "$SID" >"$ST/supervisor-session.txt"
echo "pii autonomy · supervisor ran (rc=$RC, session ${SID:0:8}) — log ${LOG##*/}"
