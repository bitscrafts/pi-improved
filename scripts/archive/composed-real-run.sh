#!/usr/bin/env bash
# One REAL run of the composed `spec` graph through the PRODUCTION devbox daemon with the real claude on haiku, in a throwaway repo.
# Level 5 of the composability programme: every earlier level used stand-in agents. This is the first time the shipped graph
# (code -> verify -> docs call -> reverify -> done) meets the real transport, the real wall clock and the real budget.
#
#   AGENT claude   MODEL claude-haiku-5-5   PROVIDER anthropic   SPEND CAP USD 1.00 (graph [limits] AND the input's max_budget_usd)
#
# The repo is a scratch git repo created under _tmp/devbox-test/ (the daemon's write sandbox is the workspace); the live project is never
# the workdir. Nothing is committed anywhere by the worker. Every printed fact is read from the daemon's own journal / CLI, never retyped.
# --control: the real docs coder is told to DELETE hello.txt; `reverify` must refuse (run failed, hello.txt gone). The control that proves the
# real run's PASS can fail, with the real transport this time.
# Usage: scripts/archive/composed-real-run.sh [--dry-run] [--control]     (--dry-run prints the plan and the input, spends nothing, starts nothing)
set -u
export PATH="/root/.local/bin:/usr/local/bin:$PATH"   # uv lives here; a background shell does not inherit the interactive PATH
REPO=$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
SSH=(ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST")
C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii
AGENT=claude; MODEL=claude-haiku-5-5; PROVIDER=anthropic; CAP_USD=1.00
DRY=0; CONTROL=0
for a in "$@"; do case "$a" in --dry-run) DRY=1;; --control) CONTROL=1;; esac; done
TS=$(date +%s); W=$REPO/_tmp/devbox-test/real-$TS; LOGD=$REPO/_tmp/claude-batchC; mkdir -p "$LOGD"; LOG=$LOGD/composed-real-$TS.log
X() { "${SSH[@]}" "podman exec -w /projects/pi-improved pii-devbox $*"; }
say() { echo "$*" | tee -a "$LOG"; }

say "== composed-real-run $TS (control=$CONTROL): agent=$AGENT model=$MODEL provider=$PROVIDER cap=\$$CAP_USD workdir=(scratch) $W/repo"
HEAD=$(git -C "$REPO" rev-parse --short=12 HEAD)
PIN=$(grep -o 'sha256 = "[0-9a-f]*"' "$REPO/graphs/spec.toml" | head -1 | cut -d'"' -f2)
DIG=$(sha256sum "$REPO/graphs/docs.toml" | cut -c1-64)
[ "$PIN" = "$DIG" ] || { say "ABORT: graphs/docs.toml digest $DIG is not the pin in spec.toml ($PIN)"; exit 2; }
grep -q 'max_micro_usd *= *1_\?000_\?000' "$REPO/graphs/spec.toml" || { say "ABORT: graphs/spec.toml carries no one-dollar [limits]"; exit 2; }
grep -q 'max_wall_ms' "$REPO/graphs/spec.toml" || { say "ABORT: graphs/spec.toml carries no wall limit (spec 290)"; exit 2; }
say "HEAD $HEAD; docs.toml digest == the pin ($DIG); spec.toml has the \$1.00 limit and a wall limit"
pgrep -af 'run-batchC|run-composition|claude ' | grep -v "grep\|flock\|composed-real" | head -3 | grep . && { say "ABORT: a worker is in flight"; exit 3; }

# the scratch repo: one committed spec whose criteria the judge reads FROM THE COMMITTED BLOB, a base commit the run is judged against
mkdir -p "$W/repo/specs"
cat > "$W/repo/specs/real.md" <<'EOF'
# Spec: write PONG

## Overview
Create one file in the repository root.

## Exit Criteria
- [ ] `test -f hello.txt`
- [ ] `grep -qx PONG hello.txt`
EOF
git -C "$W/repo" init -q && git -C "$W/repo" add -A && git -C "$W/repo" -c user.email=real@local -c user.name=real commit -qm base
BASE=$(git -C "$W/repo" rev-parse HEAD)
INNER=/projects/pi-improved/_tmp/devbox-test/real-$TS
BRIEF='Implement specs/real.md in this repository: create hello.txt in the repository root containing exactly the line PONG. Do not commit anything and do not edit specs/real.md.'
DBRIEF_OK='Documentation step. Create NOTES.md in the repository root with one line saying what hello.txt contains. Do not modify hello.txt or specs/real.md and do not commit anything.'
DBRIEF_BAD='Documentation step, with a requirement change from the owner: hello.txt must now contain the line DOCS instead of PONG, because the documentation build reads it. Overwrite hello.txt with exactly the line DOCS, then create NOTES.md saying you did. Do not commit anything.'
DBRIEF=$DBRIEF_OK; [ "$CONTROL" = 1 ] && DBRIEF=$DBRIEF_BAD
uv run --no-project python - "$W/input.json" "$AGENT" "$MODEL" "$PROVIDER" "$BASE" "$INNER/repo" "$BRIEF" "$DBRIEF" "$CAP_USD" <<'PY' || { say "ABORT: could not build the input"; exit 2; }
import json, sys
p, agent, model, provider, base, workdir, brief, dbrief, cap = sys.argv[1:]
open(p, "w").write(json.dumps({"agent": agent, "model": model, "provider": provider, "spec": "specs/real.md", "base": base,
    "workdir": workdir, "brief": brief, "docs_brief": dbrief, "max_budget_usd": float(cap)}))
PY
say "input: $(cut -c1-300 "$W/input.json")"
if [ "$DRY" = 1 ]; then say "DRY RUN: nothing started, nothing spent"; exit 0; fi

# the daemon, at HEAD (a stale daemon would run old code under a truthful label: spec 279)
DOUT=$(bash "$REPO/modules/devbox/daemon.sh" up 2>&1); DRC=$?
REV=$(printf '%s' "$DOUT" | grep -o '"revision":"[^"]*"' | tail -1 | cut -d'"' -f4)
say "DAEMON  : revision $REV (HEAD $HEAD)"
[ $DRC -eq 0 ] || { say "ABORT: the daemon is not up"; exit 2; }
case "$REV" in "$HEAD"*) ;; *) say "ABORT: the daemon's revision is not HEAD"; exit 2;; esac
X sh -c "'git -C $INNER/repo status --short >/dev/null 2>&1'" || { say "ABORT: git in the container cannot read the scratch repo (ownership?)"; exit 2; }
cp "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" "$REPO/_tmp/devbox-test/graphs/" || exit 2

cat > "$W/start.sh" <<EOF
#!/bin/sh
env PII_RUNTIME_DIR=$C/run $PIIB run start spec --input "\$(cat $INNER/input.json)" 2>&1
EOF
LINES_BEFORE=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')
START=$(X sh $INNER/start.sh)
RID=$(printf '%s' "$START" | grep -o 'r[0-9]\{12\}' | head -1)
[ -n "$RID" ] || { say "ABORT: no run id: $(printf '%s' "$START" | head -c 400)"; exit 2; }
say "RUN     : $RID"

ST=""; for i in $(seq 1 720); do
  ST=$(X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run get $RID 2>&1'" | grep -o '"status":"[a-z_]*"' | head -1)
  case "$ST" in *succeeded*|*failed*|*refused*|*cancelled*) break;; esac; sleep 5
done
say "== $RID final: ${ST:-unknown} after ~$((i*5))s"
{ echo "== explain"; X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run explain $RID 2>&1 | head -60'"
  echo "== trace";   X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run trace $RID 2>&1 | head -40'"; } >> "$LOG" 2>&1
TRACE=$(sed -n '/^== trace/,$p' "$LOG")
J=$("${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LINES_BEFORE:-0}+1)) $C/pii.log'" 2>/dev/null)
printf '%s\n' "$J" | grep -aE 'coder\.(spawned|exited)|graph\.call|quota' > "$W/journal-lines.txt"
pick() { grep -a "$1" "$W/journal-lines.txt" | sed -E "s/.*\"$2\":\"?([^,\"}]*).*/\1/"; }
SPAWNS=$(grep -ac 'coder.spawned' "$W/journal-lines.txt"); EXITS=$(grep -ac 'coder.exited' "$W/journal-lines.txt"); CALLS=$(printf '%s\n' "$TRACE" | grep -c "^call docs -> graph docs sha256 ${DIG:0:12} ")
SESS=$(pick coder.spawned session | sort -u | grep -c .)
COSTS=$(pick coder.exited claude_cost_usd | grep -E '^[0-9.]+$' | tr '\n' ' ')
TOTAL=$(printf '%s\n' $COSTS | awk '{s+=$1} END{printf "%.6f", s}')
QX=$(pick coder.exited quota_exhausted | sort -u | tr '\n' ' ')
RCS=$(pick coder.exited rc | tr '\n' ' ')
TURNS=$(pick coder.exited claude_turns | tr '\n' ' ')
HELLO=$(cat "$W/repo/hello.txt" 2>/dev/null); NOTES=$(head -c 200 "$W/repo/NOTES.md" 2>/dev/null)
say "FACTS   : spawns=$SPAWNS exits=$EXITS calls=$CALLS distinct_sessions=$SESS rc=[$RCS] turns=[$TURNS] cost=[$COSTS] total=\$$TOTAL quota_exhausted=[$QX]"
say "REPO    : hello.txt=[$HELLO]  NOTES.md=[$NOTES]  status: $(git -C "$W/repo" status --short | tr '\n' ' ')  commits: $(git -C "$W/repo" rev-list --count HEAD)"
P=0; F=0; row() { if [ "$1" = ok ]; then P=$((P+1)); say "PASS $2"; else F=$((F+1)); say "FAIL $2"; fi; }
if [ "$CONTROL" = 1 ]; then
  case "$ST" in *failed*) row ok "CONTROL: the run FAILED because the real docs coder broke the criterion ($ST)";; *) row no "CONTROL: the run did not fail ($ST)";; esac
  [ "$HELLO" = DOCS ] && row ok "CONTROL: hello.txt holds DOCS, the sabotage really happened" || row no "CONTROL: hello.txt is [$HELLO], not DOCS: nothing was sabotaged"
  [ "$SPAWNS" = 2 ] && [ "$CALLS" = 1 ] && row ok "CONTROL: both coders ran and the call is in the trace" || row no "CONTROL: spawns $SPAWNS calls $CALLS"
  printf '%s\n' "$TRACE" | grep -q 'unverified' && row ok "CONTROL: the trace reaches the unverified refusal" || row no "CONTROL: the trace never reaches unverified"
  awk -v t="$TOTAL" 'BEGIN{exit !(t>0 && t<1.0)}' && row ok "CONTROL: total cost \$$TOTAL is under the \$1.00 cap" || row no "CONTROL: total cost \$$TOTAL"
else
  case "$ST" in *succeeded*) row ok "the run succeeded ($ST)";; *) row no "the run did not succeed ($ST)";; esac
  [ "$SPAWNS" = 2 ] && [ "$EXITS" = 2 ] && row ok "two coders spawned and exited" || row no "coder spawns/exits: $SPAWNS/$EXITS (want 2/2)"
  [ "$SESS" = 2 ] && row ok "two distinct agent sessions" || row no "distinct sessions: $SESS (want 2)"
  [ "$CALLS" = 1 ] && row ok "run trace shows one call docs carrying the digest of docs.toml (${DIG:0:12})" || row no "run trace call rows with the pinned digest: $CALLS (want 1)"
  [ "$HELLO" = PONG ] && row ok "hello.txt holds PONG" || row no "hello.txt is [$HELLO]"
  [ -n "$NOTES" ] && row ok "the docs coder wrote NOTES.md" || row no "NOTES.md missing"
  [ "$(git -C "$W/repo" rev-list --count HEAD)" = 1 ] && row ok "nothing was committed by a worker" || row no "a worker committed"
  awk -v t="$TOTAL" 'BEGIN{exit !(t>0 && t<1.0)}' && row ok "total cost \$$TOTAL is under the \$1.00 cap" || row no "total cost \$$TOTAL (cap \$1.00, 0 means unreported)"
  case "$QX" in *yes*) row no "a quota refusal was journalled";; *) row ok "no quota refusal";; esac
fi
say "COMPOSED-REAL: $P pass $F fail   (evidence: $LOG, $W/journal-lines.txt)"
[ "$F" = 0 ]
