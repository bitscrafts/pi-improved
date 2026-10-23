#!/usr/bin/env bash
# modules/devbox/spec-run.sh <pi|claude> — a REAL spec implementation through a pii daemon running INSIDE the devbox.
#   pi     : ollama-cloud / deepseek-v4.1-flash:cloud   implements specs/247 (the lint requires a timeout on every coder node)
#   claude : haiku (claude-haiku-5-5)                    implements specs/246 (the agent prompt names the spec)
# The node's verdict is the run of its NAMED criterion (the spec's own test), never the agent's report. Hard-capped: timeout_ms, and for
# claude --max-budget-usd. Credentials come from the container's own environment (env file, read-only login file); nothing is read here.
# Evidence goes to _tmp/devbox-test/<agent>/ . Does NOT commit: the orchestrator reviews the diff against the spec and commits.
# A spend/usage limit in the output is printed and stops the run: no retry, no model switch.
# Needs: modules/devbox/pii-build.sh done at the current HEAD (the criteria table is compiled into the binary).
set -uo pipefail
AGENT=${1:?usage: spec-run.sh pi|claude}
HOST=${DEVBOX_HOST:-mvcorrea@10.10.10.217}
REPO=$(cd "$(dirname "$0")/../.." && pwd)            # the same tree is /projects/pi-improved inside the container
T=$REPO/_tmp/devbox-test; C=/projects/pi-improved/_tmp/devbox-test
case "$AGENT" in
  pi)     SPEC=specs/247-the-lint-requires-a-timeout-on-every-coder-node.md; CRIT=cargo:test:pii-graph/coder_requires_timeout_lint
          AG=$'agent = "pi"\nmodel = "deepseek-v4.1-flash:cloud"\nprovider = "ollama-cloud"'; BUDGET='' ;;
  claude) SPEC=specs/246-the-agent-prompt-names-the-spec.md; CRIT=cargo:test:pii-server/coder_brief_names_the_spec
          AG=$'agent = "claude"\nmodel = "haiku"'; BUDGET='max_budget_usd = 2.00' ;;
  *) echo "agent must be pi or claude"; exit 2 ;;
esac
[ -f "$REPO/$SPEC" ] || { echo "spec missing: $SPEC"; exit 2; }
if ps -eo args | grep -q '[c]laude -p'; then echo "a worker is active on this tree: not starting"; exit 3; fi
mkdir -p "$T/graphs" "$T/$AGENT"
cat > "$T/graphs/real-$AGENT.toml" <<GRAPH
name = "real-$AGENT"
entry = "code"
caps = ["spawn", "proc"]

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
$AG
workdir = "/projects/pi-improved"
spec = "$SPEC"
brief = "Implement the spec. Follow its Requirements, TDD Contract and Guardrails exactly. Write the test file first, then the code."
env_pass = ["PATH", "HOME", "IS_SANDBOX", "OLLAMA_API_KEY", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR"]
criteria = ["$CRIT"]
timeout_ms = 1500000
$BUDGET

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
git -C "$REPO" status --porcelain | grep -v 'QUEUE-BRIEF\|__pycache__' | sort > "$T/$AGENT/tree-before.txt"
echo "agent=$AGENT spec=$SPEC criterion=$CRIT"
echo "model/provider : $(grep -E '^(model|provider) ' "$T/graphs/real-$AGENT.toml" | tr '\n' ' ')"
echo "tree before    : $(wc -l < "$T/$AGENT/tree-before.txt") modified/untracked paths (recorded)"
ssh -o BatchMode=yes "$HOST" "AGENT=$AGENT C=$C bash -s" <<'REMOTE'
X() { podman exec -w /projects/pi-improved pii-devbox "$@"; }
PII=/projects/pi-improved/target/debug/pii
export_env="PII_RUNTIME_DIR=$C/run PI_CODING_AGENT_DIR=/projects/pi-improved/_tmp/pii-agent IS_SANDBOX=1 PII_MODEL=llamacpp-server:granite-4.2-3b CLAUDE_CODE_TMPDIR=$C/tmp"
# CLAUDE_CODE_TMPDIR: claude makes /tmp/claude-<uid> for its scratch and ignores TMPDIR; the daemon's landlock sandbox lets writes go to the
# workspace, store, runtime dir and $TMPDIR only, so in a fresh container claude died with "EACCES: mkdir '/tmp/claude-0'" (measured,
# r000000000002). The local daemon never hit it because that directory already existed from earlier unsandboxed runs.
# PII_MODEL: the daemon refuses to start without one ("no model configured"; measured). A coder run never calls it: the agent is pi or
# claude. The value is the model name the project's own pii.toml carries ([core].model), not a secret.
if ! X sh -c "test -S $C/run/control.sock && env PII_RUNTIME_DIR=$C/run $PII health --json >/dev/null 2>&1"; then
  X sh -c "mkdir -p $C/run $C/tmp && rm -f $C/run/control.sock"
  podman exec -d -w /projects/pi-improved pii-devbox sh -c "env $export_env TMPDIR=$C/tmp $PII daemon --foreground --store $C/store.redb --workspace /projects/pi-improved --log-file $C/pii.log > $C/daemon.out 2>&1"
  for i in $(seq 1 30); do X sh -c "env PII_RUNTIME_DIR=$C/run $PII health --json >/dev/null 2>&1" && break; sleep 1; done
fi
X sh -c "env PII_RUNTIME_DIR=$C/run $PII health --json 2>&1 | head -c 300"; echo
echo "== start run real-$AGENT"
OUT=$(X sh -c "env PII_RUNTIME_DIR=$C/run $PII run start real-$AGENT 2>&1")
echo "$OUT" | head -c 300; echo
R=$(echo "$OUT" | grep -o 'r[0-9]\{12\}' | head -1)
[ -n "$R" ] || { echo "no run id"; exit 4; }
echo "$R" > $C/$AGENT/run-id.txt
for i in $(seq 1 300); do
  ST=$(X sh -c "env PII_RUNTIME_DIR=$C/run $PII run get $R 2>&1" | grep -o '"status":"[a-z_]*"' | head -1)
  case "$ST" in *succeeded*|*failed*|*refused*|*cancelled*) break ;; esac
  [ $((i % 12)) -eq 0 ] && echo "  ... $((i*5))s $ST"
  sleep 5
done
echo "== $R final: $ST after ~$((i*5))s"
X sh -c "env PII_RUNTIME_DIR=$C/run $PII run explain $R 2>&1 | head -40" | cut -c1-200
echo "== coder.exited (journal)"
X sh -c "grep -a '\"kind\":\"coder.exited\"' $C/pii.log | tail -1" | tr ',' '\n' | grep -E 'rc|stop_reason|tree_changed|route|cost|turns|criteria|session|quota|pre_passed|stderr_head|files' | cut -c1-200
REMOTE
git -C "$REPO" status --porcelain | grep -v 'QUEUE-BRIEF\|__pycache__' | sort > "$T/$AGENT/tree-after.txt"
echo "== paths the agent changed"; comm -13 "$T/$AGENT/tree-before.txt" "$T/$AGENT/tree-after.txt"
git -C "$REPO" diff --stat | tail -4
