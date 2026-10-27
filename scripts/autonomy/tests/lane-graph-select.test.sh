#!/usr/bin/env bash
# Test: run-batchC-pii.sh selects its lane graph (spec 294). Every case uses --dry-run, which prints the lane and writes the graph or the
# input and exits BEFORE the daemon, ssh or any agent. The default one-node graph must stay byte-identical to the one the script wrote
# before the lane existed (the only difference allowed is the run's timestamp in its name). Controls: a changed pin, and a cap over the limit.
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 2
export PATH="/root/.local/bin:$PATH"
export ALLOW_DIRTY=1 DEVBOX_HOST=nobody@127.0.0.1
REPO=$(pwd); PASS=0; FAIL=0
# uv (the input builder) keeps its cache and managed Pythons under the repo's _tmp when the environment gives it no writable home
export UV_CACHE_DIR=${UV_CACHE_DIR:-$REPO/_tmp/uv-cache} UV_PYTHON_INSTALL_DIR=${UV_PYTHON_INSTALL_DIR:-$REPO/_tmp/uv-python}
S=$REPO/scripts/run-batchC-pii.sh
ok()  { echo "ok   $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
SCRATCH=$(mktemp -d "$REPO/_tmp/lane-select-XXXXXX") || exit 2
trash() { mkdir -p "$REPO/_tmp/_trash"; for f in "$@"; do [ -e "$f" ] && mv "$f" "$REPO/_tmp/_trash/lane-select-$(date +%s)-$(basename "$f")"; done; }
field() { printf '%s\n' "$1" | sed -n "s/^$2 *: //p" | head -1; } # <output> <KEY> -> the value of a "KEY    : value" line
norm() { sed 's/^name = "q294-[0-9]*"$/name = "q294-TS"/' "$1"; }

# the graph the script wrote BEFORE the lane existed (captured from the pre-294 script; brief and spec are fixed text)
cat > "$SCRATCH/golden.toml" <<'GOLDEN'
name = "q294-TS"
entry = "code"
caps = ["spawn", "proc"]

[limits]
max_wall_ms = 3300000

[[node]]
id = "code"
kind = "coder"
caps = ["spawn", "proc"]
agent = "claude"
model = "claude-haiku-5-5"
max_budget_usd = 3.00

workdir = "/projects/pi-improved"
spec = "specs/294-the-queue-worker-can-start-the-composed-spec-graph.md"
brief = '''
Implement spec 294-the-queue-worker-can-start-the-composed-spec-graph (the spec file is the CONTRACT; its path is named at the end of this prompt).
Follow it exactly: TDD (see the RED fail before the GREEN), minimal diffs.
Do NOT run the whole workspace gate: the loop runs quality-gate.sh after you and it is the arbiter. Before you finish run the cheap half:
    cargo fmt --all
    cargo clippy -p <each crate you touched> --all-targets -- -D warnings
and run only the tests your spec names (cargo test -p <crate> --test <name>). Keep the turn short.
HARD RULES: never create, edit or rewrite anything under specs/ (read-only for implementers). Do NOT commit and do not run git commit/push.
Destructive commands are banned: never rm -rf, never git checkout/restore ., git reset --hard or a bare git stash in this shared tree,
and never touch /dev, /proc, /sys or /root. Scratch you create goes under /projects/_tmp/ and only that may be removed, with a guarded script.
Work in the foreground; no background processes. If you run out of budget, write a COMPLETE handoff (done/remaining/next-start) to
/projects/pi-improved/_tmp/claude-batchC/HANDOFF-294-the-queue-worker-can-start-the-composed-spec-graph.md and stop honestly; never compress the work to fake completion.
Final line: SPEC-DONE 294 criteria=<passed>/<total>, with every criterion command and its exit code listed.
'''
env_pass = ["PATH", "HOME", "IS_SANDBOX", "OLLAMA_API_KEY", "PI_CODING_AGENT_DIR", "CLAUDE_CODE_TMPDIR"]
timeout_ms = 3000000

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
GOLDEN

echo "== default: no PII_LANE_GRAPH, dry run"
OUT=$(timeout 60 bash "$S" 294 --dry-run 2>&1); RC=$?
G=$(field "$OUT" GRAPH)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q '^LANE-GRAPH: coder$' && [ -f "$G" ]; then
  if diff <(norm "$G") <(norm "$SCRATCH/golden.toml") >/dev/null; then ok "default_is_the_one_node_graph: LANE-GRAPH coder, graph identical to the pre-294 graph"
  else bad "default_is_the_one_node_graph: the graph differs from the pre-294 graph"; diff <(norm "$G") <(norm "$SCRATCH/golden.toml") | head -20; fi
  trash "$G"
else bad "default_is_the_one_node_graph: exit $RC, LANE-GRAPH line or graph missing"; printf '%s\n' "$OUT" | head -20; fi

echo "== spec lane: PII_LANE_GRAPH=spec, dry run"
OUT=$(PII_LANE_GRAPH=spec timeout 60 bash "$S" 294 --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -q '^LANE-GRAPH: spec$' && [ -f "$I" ]; then
  KEYS=$(uv run --no-project python -I -c 'import json,sys; print(" ".join(sorted(json.load(open(sys.argv[1])))))' "$I" 2>&1)
  WANT="agent base brief docs_brief max_budget_usd model provider spec workdir"
  if [ "$KEYS" = "$WANT" ]; then ok "spec_graph_is_selected: LANE-GRAPH spec, input with all nine keys"
  else bad "spec_graph_is_selected: input keys [$KEYS], want [$WANT]"; fi
  trash "$I"
else bad "spec_graph_is_selected: exit $RC, LANE-GRAPH line or input missing"; printf '%s\n' "$OUT" | head -20; fi

echo "== resume with the spec lane falls back to the one-node graph"
OUT=$(PII_LANE_GRAPH=spec timeout 60 bash "$S" 294 --resume abc --dry-run 2>&1); RC=$?
if [ "$RC" = 0 ] && printf '%s\n' "$OUT" | grep -qx 'LANE-GRAPH: coder (resume is not supported by the spec graph)' \
   && printf '%s\n' "$OUT" | grep -q '^LANE-GRAPH: coder$'; then ok "resume_falls_back: fallback line and LANE-GRAPH coder"
else bad "resume_falls_back: exit $RC"; printf '%s\n' "$OUT" | head -20; fi
trash "$(field "$OUT" GRAPH)"

# the controls run the script from a scratch repo: its REPO is rewritten, so the real graphs/ and specs/ are never touched
mkdir -p "$SCRATCH/repo/graphs" "$SCRATCH/repo/specs"
cp "$REPO/graphs/spec.toml" "$REPO/graphs/docs.toml" "$SCRATCH/repo/graphs/"
cp "$REPO"/specs/294-*.md "$SCRATCH/repo/specs/"
sed "s|^REPO=/projects/pi-improved|REPO=$SCRATCH/repo|" "$S" > "$SCRATCH/repo/run.sh"

echo "== control: the unchanged pin is accepted (proves the refusal below is not a broken script)"
OUT=$(PII_LANE_GRAPH=spec timeout 60 bash "$SCRATCH/repo/run.sh" 294 --dry-run 2>&1); RC=$?
if [ "$RC" = 0 ]; then ok "control_unchanged_pin_is_accepted: exit 0"; else bad "control_unchanged_pin_is_accepted: exit $RC"; printf '%s\n' "$OUT" | head -10; fi

echo "== control: docs.toml with one byte changed"
printf ' ' >> "$SCRATCH/repo/graphs/docs.toml"
OUT=$(PII_LANE_GRAPH=spec timeout 60 bash "$SCRATCH/repo/run.sh" 294 --dry-run 2>&1); RC=$?
if [ "$RC" = 2 ] && printf '%s\n' "$OUT" | grep -q '^ABORT' && ! printf '%s\n' "$OUT" | grep -qE '^(DAEMON|RUN|DRY RUN)'; then ok "CONTROL_a_changed_pin_is_refused: exit 2, nothing started"
else bad "CONTROL_a_changed_pin_is_refused: exit $RC"; printf '%s\n' "$OUT" | head -10; fi

echo "== control: PII_MAX_BUDGET_USD=9 is capped at the graph's own limit"
cp "$REPO/graphs/docs.toml" "$SCRATCH/repo/graphs/docs.toml"
OUT=$(PII_LANE_GRAPH=spec PII_MAX_BUDGET_USD=9 timeout 60 bash "$S" 294 --dry-run 2>&1); RC=$?
I=$(field "$OUT" INPUT)
CAP=$( [ -f "$I" ] && uv run --no-project python -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["max_budget_usd"])' "$I" 2>/dev/null)
if [ "$RC" = 0 ] && [ "$CAP" = 1.0 ]; then ok "CONTROL_cap_never_exceeds_the_limit: max_budget_usd 1.0"
else bad "CONTROL_cap_never_exceeds_the_limit: exit $RC, max_budget_usd [$CAP], want 1.0"; fi
trash "$I" "$(field "$OUT" GRAPH)"

trash "$SCRATCH"
echo "PASS: $PASS FAIL: $FAIL"
