#!/usr/bin/env bash
# run-batchC-pii.sh — dispatch ONE queue item through PII: a coder-node graph run by the pii daemon inside the devbox container on the
# lnx host (modules/devbox). User directive 2026-10-24: "from now on all our specs should be implemented by pii".
#
# Same contract as run-batchC-claude.sh, so cycle.sh swaps one for the other (PII_WORKER=pii, the default):
#   - prints AGENT, MODEL, PROVIDER, SPEC, BRIEF, LOG, GRAPH before anything is spent
#   - sets its own environment (HARD RULE 0d); no credential is read or printed here (the container holds them)
#   - records the agent's session handle to _tmp/claude-batchC/session-<spec>.txt BEFORE the run ends (spec 244: pii journals it at spawn)
#   - resume-first: `--resume <sid>` continues the agent's own session when the container still has it
#   - never commits; the spec files are read-only for the worker (verified AFTER the run: pii's argv has no --disallowedTools)
#   - a provider quota refusal is written to the log in the CLI's own words, so scripts/autonomy/window.sh parses it unchanged
#
# Usage:  bash scripts/run-batchC-pii.sh <spec-number> [--resume <session-id>] [extra-brief-file]
# Env:    PII_AGENT   claude (default) | pi
#         CLAUDE_MODEL (default claude-haiku-5-5)   PI_MODEL (default deepseek-v4.1-flash:cloud)   PI_PROVIDER (default ollama-cloud)
#         PII_NODE_TIMEOUT_MS (default 3000000 = 50 min)   PII_MAX_BUDGET_USD (claude only, default 3.00)
#         ALLOW_DIRTY=1 required when the tree has the item's own WIP (cycle.sh sets it)
# Exit:   0 run succeeded and specs/ untouched | 1 run failed | 2 setup/daemon | 8 dirty tree | 9 the agent edited specs/
set -uo pipefail
export PATH="/root/.local/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
REPO=/projects/pi-improved
LOGD=$REPO/_tmp/claude-batchC
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
# PII_SSH names the ssh binary; the lane puts /usr/bin ahead of PATH, so a test cannot shadow `ssh` through PATH (measured: the stub was never run). Default: ssh.
SSH=("${PII_SSH:-ssh}" -o BatchMode=yes -o ConnectTimeout=15 "$HOST")
cd "$REPO" || exit 2
mkdir -p "$LOGD" "$REPO/_tmp/devbox-test/graphs"

N="${1:-}"; [ -n "$N" ] || { echo "usage: run-batchC-pii.sh <spec-number> [--resume <session-id>] [extra-brief-file]"; exit 2; }
SPEC=$(ls "$REPO"/specs/"$N"-*.md 2>/dev/null | head -1); [ -f "$SPEC" ] || { echo "no spec $N"; exit 2; }
TAG=$(basename "$SPEC" .md); SPECREL=specs/$(basename "$SPEC")
shift || true
RESUME_SID=""; EXTRA_BRIEF=""; DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --resume) RESUME_SID="${2:?--resume needs a session id}"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    *) EXTRA_BRIEF="$1"; shift ;;
  esac
done
if [ -n "$(git status --porcelain)" ] && [ -z "${ALLOW_DIRTY:-}" ]; then
  echo "STOPPED: dirty tree (set ALLOW_DIRTY=1 only to resume this spec's own WIP):"; git status --porcelain; exit 8
fi

AGENT=${PII_AGENT:-claude}
case "$AGENT" in
  claude) MODEL=${CLAUDE_MODEL:-claude-haiku-5-5}; PROVIDER="anthropic (the claude login file inside the devbox)"
          AGENT_LINES=$'agent = "claude"\nmodel = "'"$MODEL"$'"\nmax_budget_usd = '"${PII_MAX_BUDGET_USD:-3.00}" ;;
  pi)     MODEL=${PI_MODEL:-deepseek-v4.1-flash:cloud}; PROVIDER=${PI_PROVIDER:-ollama-cloud}
          AGENT_LINES=$'agent = "pi"\nmodel = "'"$MODEL"$'"\nprovider = "'"$PROVIDER"'"' ;;
  *) echo "PII_AGENT must be claude or pi"; exit 2 ;;
esac
TS=$(date +%s); LOG="$LOGD/$TAG.$TS.log"
GRAPH_NAME="q$N-$TS"; GRAPH="$REPO/_tmp/devbox-test/graphs/$GRAPH_NAME.toml"
BRIEF="$LOGD/brief-$TAG.md"
# ---- the lane (spec 294): PII_LANE_GRAPH=spec starts the tracked `spec` graph; anything else keeps the one-node coder graph below.
# The spec graph has no session input, so a resume stays on the one-node graph. Decided before the brief, which names the scratch. Printed before anything is spent.
LANE=coder
if [ "${PII_LANE_GRAPH:-}" = spec ]; then
  if [ -n "$RESUME_SID" ]; then echo "LANE-GRAPH: coder (resume is not supported by the spec graph)"
  else LANE=spec; GRAPH_NAME=spec; fi
fi
echo "LANE-GRAPH: $LANE"

# the scratch the brief names (spec 301): the spec lane's is _tmp inside the repository, which the daemon's judge may write (the workspace landlock
# profile); the coder lane keeps /projects/_tmp, so its brief and graph stay byte-identical. The preflight proves the scratch through the daemon.
if [ "$LANE" = spec ]; then SCRATCH_TEXT="_tmp/ in the repository"; SCRATCH=$REPO/_tmp; else SCRATCH_TEXT="/projects/_tmp/"; SCRATCH=/projects/_tmp; fi
PREFLIGHT_CMD="command -v uv >/dev/null"

# ---- the brief: what the old worker was told, minus the flag-enforced parts (those are verified below instead)
cat > "$BRIEF" <<EOF
Implement spec $TAG (the spec file is the CONTRACT; its path is named at the end of this prompt).
Follow it exactly: TDD (see the RED fail before the GREEN), minimal diffs.
Do NOT run the whole workspace gate: the loop runs quality-gate.sh after you and it is the arbiter. Before you finish run the cheap half:
    cargo fmt --all
    cargo clippy -p <each crate you touched> --all-targets -- -D warnings
and run only the tests your spec names (cargo test -p <crate> --test <name>). Keep the turn short.
HARD RULES: never create, edit or rewrite anything under specs/ (read-only for implementers). Do NOT commit and do not run git commit/push.
Destructive commands are banned: never rm -rf, never git checkout/restore ., git reset --hard or a bare git stash in this shared tree,
and never touch /dev, /proc, /sys or /root. Scratch you create goes under $SCRATCH_TEXT and only that may be removed, with a guarded script.
Work in the foreground; no background processes. If you run out of budget, write a COMPLETE handoff (done/remaining/next-start) to
$LOGD/HANDOFF-$TAG.md and stop honestly; never compress the work to fake completion.
Final line: SPEC-DONE $N criteria=<passed>/<total>, with every criterion command and its exit code listed.
EOF
[ -n "$EXTRA_BRIEF" ] && { printf '\n\n' >> "$BRIEF"; cat "$EXTRA_BRIEF" >> "$BRIEF"; }
if grep -q "'''" "$BRIEF"; then echo "ABORT: the brief contains ''' and cannot be written as a TOML literal"; exit 2; fi

# ---- the spec lane's wait (spec 299): the deadline is the graph's own wall in seconds plus a margin, measured with SECONDS. Read here, so a
# graph without a numeric wall aborts before anything is started. PII_WAIT_MARGIN_S (default 600) exists so a test can shorten the margin.
if [ "$LANE" = spec ]; then
  SPEC_WALL_MS=$(sed -n 's/^max_wall_ms *= *\([0-9][0-9]*\).*/\1/p' "$REPO/graphs/spec.toml" | head -1)
  case "$SPEC_WALL_MS" in ''|*[!0-9]*) echo "ABORT: graphs/spec.toml has no numeric max_wall_ms: nothing started"; exit 2 ;; esac
  WAIT_MARGIN_S=${PII_WAIT_MARGIN_S:-600}
  DEADLINE=$(( SPEC_WALL_MS / 1000 + WAIT_MARGIN_S ))
fi

# ---- resume handle: claude's own session, only if the container still has it (the old lane's sessions live elsewhere)
SID_LINE=""
if [ -n "$RESUME_SID" ] && [ "$AGENT" = claude ]; then
  if [ "$DRY" = 1 ]; then SID_LINE="sid = \"$RESUME_SID\""; echo "== dry run: session $RESUME_SID is not checked in the devbox"
  elif "${SSH[@]}" "podman exec pii-devbox sh -c 'ls /root/.claude/projects/*/$RESUME_SID.jsonl >/dev/null 2>&1'"; then
    SID_LINE="sid = \"$RESUME_SID\""; echo "== RESUMING claude session $RESUME_SID (found in the devbox)"
  else
    echo "== NOT resuming: session $RESUME_SID is not in the devbox (it was made by another lane); starting fresh on the tree as it is"
  fi
fi

# ---- the graph: one coder node; no criteria (the cycle runs the row's own criteria next), so the node's verdict is the agent's exit
# and the journal's tree_changed / quota facts. `refuse` makes an agent failure FAIL the run (spec 240), never succeed it.
# The run's wall limit must outlive its node: the engine's default is 600 000 ms and it is checked before the NEXT node is dispatched,
# so a coder that ran past 10 minutes and exited 0 had its run FAILED (measured: 601 s and 717 s failed, 382 s succeeded).
if [ "$LANE" = coder ]; then
WALL_MS=$(( ${PII_NODE_TIMEOUT_MS:-3000000} + 300000 ))
cat > "$GRAPH" <<GRAPH
name = "$GRAPH_NAME"
entry = "code"
caps = ["spawn", "proc"]

[limits]
max_wall_ms = $WALL_MS

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
$AGENT_LINES
$SID_LINE
workdir = "$REPO"
spec = "$SPECREL"
brief = '''
$(cat "$BRIEF")
'''
env_pass = ["PATH", "HOME", "IS_SANDBOX", "OLLAMA_API_KEY", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR"]
timeout_ms = ${PII_NODE_TIMEOUT_MS:-3000000}

[[node]]
id = "done"
kind = "transform"
value = "done"

[[node]]
id = "refused"
kind = "refuse"
hook = "coder"
reason = "the agent failed"

[[edge]]
from = "code"
port = "ok"
to = "done"
kind = "data"

[[edge]]
from = "code"
port = "err"
to = "refused"
kind = "data"
GRAPH
fi

# ---- the spec lane: the input of the tracked graph. The pin in graphs/spec.toml must be the digest of graphs/docs.toml (refused otherwise);
# max_budget_usd is the queue's PII_MAX_BUDGET_USD (default 1.00), never above the graph's own [limits] max_micro_usd.
if [ "$LANE" = spec ]; then
  PIN=$(grep -o 'sha256 = "[0-9a-f]*"' "$REPO/graphs/spec.toml" | head -1 | cut -d'"' -f2)
  DIG=$(sha256sum "$REPO/graphs/docs.toml" 2>/dev/null | cut -c1-64)
  [ -n "$DIG" ] && [ "$PIN" = "$DIG" ] || { echo "ABORT: graphs/docs.toml digest '$DIG' is not the pin in graphs/spec.toml ('$PIN'): nothing started"; exit 2; }
  [ -f "$REPO/graphs/preflight.toml" ] || { echo "ABORT: graphs/preflight.toml is missing (the preflight judge graph): nothing started"; exit 2; }
  IN_PROVIDER=$PROVIDER; [ "$AGENT" = claude ] && IN_PROVIDER=anthropic
  DOCS_BRIEF="Documentation step. Create the file _tmp/docs-notes/NOTES-$N.md (make the directory first) with a short note on what the spec changed. Do not modify any other file, do not modify anything under specs/, and do not commit anything."
  INPUT="$LOGD/input-$TAG.json"
  uv run --no-project python - "$INPUT" "$AGENT" "$MODEL" "$IN_PROVIDER" "$SPECREL" "$(git rev-parse HEAD)" "$REPO" "$BRIEF" "$DOCS_BRIEF" \
    "${PII_MAX_BUDGET_USD:-1.00}" "$REPO/graphs/spec.toml" <<'PY' || { echo "ABORT: could not build the input"; exit 2; }
import json, sys, tomllib
p, agent, model, provider, spec, base, workdir, brief, docs_brief, cap, graph = sys.argv[1:]
limit = tomllib.load(open(graph, "rb"))["limits"]["max_micro_usd"] / 1e6
open(p, "w").write(json.dumps({"agent": agent, "model": model, "provider": provider, "spec": spec, "base": base, "workdir": workdir,
    "brief": open(brief).read(), "docs_brief": docs_brief, "max_budget_usd": min(float(cap), limit)}))
PY
fi

echo "AGENT    : $AGENT   (run by pii, not by the agent CLI directly)"
echo "MODEL    : $MODEL"
echo "PROVIDER : $PROVIDER"
echo "SPEC     : $SPECREL"
echo "BRIEF    : $BRIEF ($(wc -c < "$BRIEF") bytes)"
if [ "$LANE" = spec ]; then echo "INPUT    : $INPUT"; else echo "GRAPH    : $GRAPH"; fi
echo "LOG      : $LOG"
[ "$LANE" = spec ] && echo "WAIT     : up to ${DEADLINE}s (graph max_wall_ms $SPEC_WALL_MS + $WAIT_MARGIN_S)"
{ echo "== $TAG via pii: agent=$AGENT model=$MODEL provider=$PROVIDER"; echo "== graph $GRAPH_NAME"; } > "$LOG"

# ---- the container preflight (spec 298, 301): uv must exist in the devbox (a criterion may call it), checked here; the scratch is proven by
# the judge run below, after the daemon is up. A missing one refuses the run BEFORE anything is spent (the judges run the criteria in the container)
if [ "$LANE" = spec ]; then
  if [ "$DRY" = 1 ]; then echo "PREFLIGHT: skipped (dry run)"
  else
    PRE=$("${SSH[@]}" "podman exec pii-devbox sh -c '$PREFLIGHT_CMD'" 2>&1) || { echo "ABORT: container preflight failed: ${PRE:-no output}"; exit 2; }
    echo "PREFLIGHT: uv ok"
  fi
fi
[ "$DRY" = 1 ] && { echo "DRY RUN: nothing started, nothing spent"; exit 0; }

# ---- the daemon: up, and its build revision stated (it runs a deliberate binary, see modules/devbox/daemon.sh)
# PII_DAEMON_SH: a test supplies a stub; the default is the real devbox script (it uses its own ssh, which PII_SSH does not reach)
DOUT=$(bash "${PII_DAEMON_SH:-$REPO/modules/devbox/daemon.sh}" up 2>&1); DRC=$?
echo "DAEMON   : $(printf '%s' "$DOUT" | tail -1)"
[ $DRC -eq 0 ] || { echo "ABORT: the devbox daemon is not up:"; printf '%s\n' "$DOUT" | tail -5; exit 2; }
[ "$LANE" = spec ] && { cp "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" "$REPO/graphs/preflight.toml" "$REPO/_tmp/devbox-test/graphs/" || exit 2; }

git status --porcelain -- specs | sort > "$LOGD/.specs-before-$TAG"
C=/projects/pi-improved/_tmp/devbox-test; PIIB=$C/bin/pii
X() { "${SSH[@]}" "podman exec -w /projects/pi-improved pii-devbox $*"; }
poll() { X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run get ${1:-$RID} 2>&1'" | grep -o '"status":"[a-z_]*"' | head -1; }

# ---- the judge preflight (spec 301): a one-node judge run through the daemon, the same sandbox as the real judge. Its one criterion makes a
# directory under the scratch in a throwaway repo; the run must end succeeded before the real run starts. No agent, no spend.
if [ "$LANE" = spec ]; then
  PTS=$(date +%s); PDIR=$(mktemp -d "$REPO/_tmp/devbox-test/preflight-$PTS-XXXXXX") || { echo "ABORT: no preflight directory: nothing started"; exit 2; }
  PREL=_tmp/preflight-$PTS; PDIR_NAME=$(basename "$PDIR")
  mkdir -p "$PDIR/repo/specs" && printf '# Spec: preflight\n\n## Exit Criteria\n- [ ] `mkdir -p %s && test -d %s`\n' "$PREL" "$PREL" > "$PDIR/repo/specs/preflight.md" \
    && git -C "$PDIR/repo" init -q && git -C "$PDIR/repo" add specs \
    && git -C "$PDIR/repo" -c user.name=pii-preflight -c user.email=preflight@localhost commit -qm base || { echo "ABORT: could not make the preflight repo: nothing started"; exit 2; }
  PIN_IN="$LOGD/judge-input-$PTS.json"
  printf '{"spec":"specs/preflight.md","base":"%s","workdir":"%s"}\n' "$(git -C "$PDIR/repo" rev-parse HEAD)" "$C/$PDIR_NAME/repo" > "$PIN_IN"
  PRID=$(X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start preflight --input \"\$(cat $PIN_IN)\" 2>&1'" | grep -o 'r[0-9]\{12\}' | head -1)
  [ -n "$PRID" ] || { echo "ABORT: the preflight judge did not start: nothing started"; exit 2; }
  PST=""
  for _ in $(seq 1 60); do
    PST=$(poll "$PRID")
    case "$PST" in *succeeded*|*failed*|*refused*|*cancelled*) break ;; esac
    sleep "${PII_POLL_SECONDS:-5}"
  done
  case "$PST" in *succeeded*) echo "PREFLIGHT: the daemon's judge wrote $SCRATCH ok ($PRID)" ;; *)
    echo "ABORT: container preflight failed: the daemon's judge cannot write $SCRATCH ($PRID: ${PST:-no status})"
    X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run trace $PRID 2>&1'" | grep -E 'crit=|fail='
    exit 2 ;;
  esac
fi

LINES_BEFORE=$("${SSH[@]}" "podman exec pii-devbox sh -c 'wc -l < $C/pii.log'" 2>/dev/null | tr -d ' ')
if [ "$LANE" = spec ]; then
  printf '#!/bin/sh\nenv PII_RUNTIME_DIR=%s %s run start spec --input "$(cat %s)" 2>&1\n' "$C/run" "$PIIB" "$INPUT" > "$LOGD/start-$TAG.sh"
  START=$(X sh "$LOGD/start-$TAG.sh")
else
  START=$(X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run start $GRAPH_NAME 2>&1'")
fi
RID=$(printf '%s' "$START" | grep -o 'r[0-9]\{12\}' | head -1)
[ -n "$RID" ] || { echo "ABORT: no run id: $(printf '%s' "$START" | head -c 300)"; exit 2; }
echo "RUN      : $RID"; echo "== run $RID" >> "$LOG"

# ---- the session handle, the moment pii journals it (coder.spawned), before the agent can finish
SID=""
for _ in $(seq 1 60); do
  SID=$("${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LINES_BEFORE:-0}+1)) $C/pii.log | grep -a coder.spawned | tail -1'" 2>/dev/null | grep -o '"session":"[^"]*"' | head -1 | cut -d'"' -f4)
  [ -n "$SID" ] && break; sleep 2
done
[ -n "$SID" ] && { printf '%s\n' "$SID" > "$LOGD/session-$TAG.txt"; echo "== session handle captured at spawn: $SID"; }

# ---- wait for the run. The engine serialises a cancel as `failed`; `cancelled` stays a terminal word too (spec 299, no behaviour change).
ST=""
if [ "$LANE" = spec ]; then
  # spec 299: the wait is the graph's wall (DEADLINE, read above), not an iteration count
  SECONDS=0
  while :; do
    ST=$(poll)
    case "$ST" in *succeeded*|*failed*|*refused*|*cancelled*) break ;; esac
    [ "$SECONDS" -ge "$DEADLINE" ] && break
    sleep "${PII_POLL_SECONDS:-5}"
  done
  # the engine never cancels a run because its client stopped polling, so the lane cancels it once it gives up (spec 299)
  case "$ST" in *succeeded*|*failed*|*refused*|*cancelled*) ;; *)
    CANC=$(X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run cancel $RID --kill --reason lane-wait-expired 2>&1'")
    { echo "== $RID still running after ${SECONDS}s: cancelled by the lane"; printf '%s\n' "$CANC"; } | tee -a "$LOG" ;;
  esac
  WAITED=$SECONDS
else
  # bounded a little past the node's own timeout
  MAXI=$(( ${PII_NODE_TIMEOUT_MS:-3000000} / 5000 + 120 ))
  for i in $(seq 1 "$MAXI"); do
    ST=$(poll)
    case "$ST" in *succeeded*|*failed*|*refused*|*cancelled*) break ;; esac
    sleep "${PII_POLL_SECONDS:-5}"
  done
  WAITED=$((i*${PII_POLL_SECONDS:-5}))
fi
echo "== $RID final: ${ST:-unknown} after ~${WAITED}s" | tee -a "$LOG"

# ---- the evidence, in the log: explain, the journal's coder.exited (raw), and the quota refusal in the CLI's own words
X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run explain $RID 2>&1 | head -60'" >> "$LOG" 2>&1
# a refused spec run says WHY (spec 298): the criteria its trace names. explain names none, so the reason comes from the trace
if [ "$LANE" = spec ]; then case "$ST" in *succeeded*) ;; *)
  WHY=$(X sh -c "'env PII_RUNTIME_DIR=$C/run $PIIB run trace $RID 2>&1'" | grep -E 'crit=|fail=')
  if [ -n "$WHY" ]; then { echo "== why the run was not accepted:"; printf '%s\n' "$WHY"; } | tee -a "$LOG"
  else echo "== why the run was not accepted: (the trace names no criterion)" | tee -a "$LOG"; fi ;;
esac; fi
EXITS=$("${SSH[@]}" "podman exec pii-devbox sh -c 'tail -n +$((${LINES_BEFORE:-0}+1)) $C/pii.log | grep -a coder.exited'" 2>/dev/null)
# the spec graph runs two coders (code, then the inlined docs): the facts are the FIRST exit (the host's code), the docs coder's is a second line
if [ "$LANE" = spec ]; then EXITED=$(printf '%s\n' "$EXITS" | head -1); DOCS_EXITED=$(printf '%s\n' "$EXITS" | sed -n 2p)
else EXITED=$(printf '%s\n' "$EXITS" | tail -1); fi
printf '== coder.exited: %s\n' "$EXITED" >> "$LOG"
fact() { printf '%s' "$1" | grep -o "\"$2\":\"\?[^,\"}]*" | head -1 | sed -E 's/^"[a-z_]*":"?//'; }
RC=$(fact "$EXITED" rc); CHG=$(fact "$EXITED" tree_changed); QX=$(fact "$EXITED" quota_exhausted); QR=$(printf '%s' "$EXITED" | grep -o '"quota_reset":"[^"]*"' | cut -d'"' -f4)
COST=$(printf '%s' "$EXITED" | grep -o '"claude_cost_usd":"[^"]*"' | cut -d'"' -f4)
# spec 300: a refusal by EITHER coder is the run's refusal (the docs coder is the second line). The reset and the name come from the refusing coder
QBY=host; QRESET=$QR
if [ "$LANE" = spec ] && [ "$QX" != yes ] && [ "$(fact "$DOCS_EXITED" quota_exhausted)" = yes ]; then
  QBY=docs; QRESET=$(printf '%s' "$DOCS_EXITED" | grep -o '"quota_reset":"[^"]*"' | cut -d'"' -f4)
fi
echo "FACTS    : rc=${RC:-?} tree_changed=${CHG:-?} quota_exhausted=${QX:-?} cost_usd=${COST:-not-journalled}"
if [ "$LANE" = spec ]; then
  DCOST=$(printf '%s' "$DOCS_EXITED" | grep -o '"claude_cost_usd":"[^"]*"' | cut -d'"' -f4)
  echo "DOCS-FACTS: rc=$(fact "$DOCS_EXITED" rc) tree_changed=$(fact "$DOCS_EXITED" tree_changed) quota_exhausted=$(fact "$DOCS_EXITED" quota_exhausted) cost_usd=${DCOST:-not-journalled}"
fi
if [ "$QX" = "yes" ] || [ "$QBY" = docs ]; then
  T=$(printf '%s' "$QRESET" | grep -oE '[0-9]{1,2}:[0-9]{2}(am|pm)' | head -1)
  if [ -n "$T" ]; then echo "You've hit your session limit · resets $T (UTC)" | tee -a "$LOG"; else echo "You've hit your session limit" | tee -a "$LOG"; fi
  echo "QUOTA    : refused by the $QBY coder" | tee -a "$LOG"
fi

# ---- specs are read-only for the worker: the old lane enforced it with a flag, this one checks the result
git status --porcelain -- specs | sort > "$LOGD/.specs-after-$TAG"
TOUCHED=$(comm -13 "$LOGD/.specs-before-$TAG" "$LOGD/.specs-after-$TAG")
if [ -n "$TOUCHED" ]; then echo "SPEC-GUARD: the agent changed files under specs/ (read-only for implementers):"; echo "$TOUCHED" | tee -a "$LOG"; exit 9; fi

echo "== tree after: $(git status --porcelain | wc -l) dirty paths"; git status --porcelain | head -12
case "$ST" in *succeeded*) exit 0 ;; *) exit 1 ;; esac
